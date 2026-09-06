//! bug-453: riscv64 unconditional-jump relaxation (hop-chain insertion).
//!
//! RISC-V's `jal` encodes its target as a 20-bit signed half-word offset
//! (`imm20`), reaching only ±1 MiB, and `jal` is the **widest single-instruction
//! jump the base ISA has**. The shared two-pass encoder
//! ([`crate::arch::encode_plan`]) validates that reach in
//! [`super::emitter::Encoder::patch_labels`] and — correctly — refuses to mask an
//! out-of-range displacement to a wrong target. But refusing is not the only
//! option: a large function whose jump must span more than ±1 MiB is *legal* and
//! should compile. AArch64 got this in bug-445
//! ([`crate::arch::aarch64::encode::relax_conditional_branches`]); this pass is
//! riscv64's equivalent.
//!
//! **Two plan ops reach the encoder as a label-relative `jal`, and both overflow
//! at the same threshold:**
//!
//! * [`CodeOp::Branch`] (`b <label>`) → `jal zero, label` at the instruction's own
//!   offset. This is the one the bug reports (`jal ... to 'trap_0' exceeds
//!   ±1 MiB`).
//! * [`CodeOp::RvBr`] (`rv.br`) → the 8-byte long form `b<inverse> rs1, rs2, +8;
//!   jal zero, label`. Its escape hatch **is** a `jal`, four bytes into the
//!   instruction, so a conditional branch across the same boundary fails
//!   identically through a different emitter path. Relaxing only `b` would make
//!   the reported reproduction pass while still rejecting functions of the same
//!   size.
//!
//! # Why a hop chain and not `auipc`+`jalr`
//!
//! Unlike AArch64 — where the veneer's unconditional `b` (`imm26`, ±128 MiB) has
//! *wider* reach than the `b.<cond>` it replaces — riscv64 has nothing wider to
//! relax into. Reaching further than ±1 MiB needs `auipc rd, %pcrel_hi(t)` +
//! `jalr zero, %pcrel_lo(t)(rd)`, and `auipc` needs a destination **register**
//! that is dead at the rewrite site. There is none: `t0`–`t2` are reserved
//! *lowering* scratch (`super::super::select`), `gp` is the plan-99 flag register,
//! and bug-381 established that rv64 has no other free register (`tp` faults a
//! dynamically-linked binary through TLS, and shrinking the allocatable pool
//! destabilizes the allocator). The `RvBr` case makes that decisive: its `jal`
//! sits *inside* an expansion where `t0`–`t2` liveness is not knowable here.
//!
//! `jal zero, offset` writes no link register, so a **jump** is register-free —
//! only its *reach* is limited. So relax by chaining hops rather than widening the
//! instruction:
//!
//! ```text
//!     jal zero, far        ==>     jal zero, Lhop      ; <= HOP bytes away
//!     ...                          ...
//!                                  jal zero, Lover     ; skip the island
//!                                Lhop:
//!                                  jal zero, far       ; <= HOP bytes onward
//!                                Lover:
//!                                  ...
//! ```
//!
//! The island must be spliced **between** the source and the target — an
//! *adjacent* trampoline (bug-445's shape) sits at essentially the same distance
//! from the target and is equally out of range. An island is an unreachable
//! sequence in the middle of the instruction stream, so it opens with a `jal zero,
//! Lover` that jumps over it; it clobbers no register and no condition state, so
//! it is transparent at any instruction boundary. When one hop is not enough the
//! chain continues: island *j* jumps to island *j+1*, and only the last one names
//! the original target.
//!
//! # No-op in range
//!
//! Relaxing a branch never changes the **size** of the branch itself — only its
//! `target` field is rewritten, and the 8-byte island is spliced elsewhere. A
//! function with no out-of-range jump is left byte-for-byte untouched, so every
//! existing `linux-riscv64` golden is unaffected.

use std::collections::HashMap;

use super::sizing::instruction_size;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::{CodeInstruction, NativeCodePlan};

/// `imm20` reach in bytes: ±2^19 half-words × 2 = ±1 MiB. Mirrors the bound
/// [`super::emitter::Encoder::patch_labels`] enforces, so a jump this pass leaves
/// alone is exactly one the encoder accepts.
const JAL_LIMIT: isize = 1 << 20;

/// How far one hop is allowed to carry. Half the hardware reach, which leaves
/// 512 KiB of slack to absorb the offset shifts the pass's own insertions cause:
/// every island is 8 bytes, so the slack only runs out after 65_536 islands in a
/// single function — and the fixpoint loop catches even that, because a jump that
/// is still out of range after a round is simply relaxed again.
const HOP: isize = JAL_LIMIT / 2;

/// Where the label-relative `jal` sits inside an instruction's encoding, in
/// bytes from the instruction's own offset. `b` *is* the `jal`; `rv.br` emits a
/// 4-byte inverted short branch first and the `jal` second. Returns `None` for
/// every op that is not a label-relative `jal`.
fn jal_offset_within(op: CodeOp) -> Option<usize> {
    match op {
        CodeOp::Branch => Some(0),
        CodeOp::RvBr => Some(4),
        _ => None,
    }
}

/// Rewrite every out-of-`imm20`-range `jal` in the plan into a hop chain so the
/// whole plan encodes. A strict no-op — the instruction stream is left
/// byte-for-byte unchanged — when every jump already fits, which is the case for
/// every program that compiled before bug-453.
pub(crate) fn relax_rv64_branches(plan: &mut NativeCodePlan) -> Result<(), String> {
    // `-vv` (`crate::trace`): the pass rewrites nothing for a normal program but
    // still scans every instruction in the plan, so it is worth its own row rather
    // than hiding in the "encoding image" stage's self time.
    let _span = crate::trace::span("relax rv64 branches");
    // One monotonic counter across the whole plan keeps every synthesized
    // hop/continuation label globally unique (labels are function-local, but a
    // shared counter is simplest and still unique).
    let mut counter = 0usize;
    for function in &mut plan.functions {
        relax_function(&mut function.instructions, &function.name, &mut counter)?;
    }
    Ok(())
}

/// One island: the jump-over, the hop label, the onward jump, and the
/// continuation label. Eight emitted bytes (two `jal`s; a label emits nothing).
fn island(hop: &str, over: &str, far: &str) -> Vec<CodeInstruction> {
    vec![
        CodeInstruction::new("b").field("target", over),
        CodeInstruction::new("label").field("name", hop),
        CodeInstruction::new("b").field("target", far),
        CodeInstruction::new("label").field("name", over),
    ]
}

/// Point an already-built branch at a different label. The op keeps every other
/// field (`rv.br` carries `lhs`/`rhs`/`cond`, which must survive), so this
/// rewrites the `target` entry in place rather than rebuilding the instruction.
fn retarget(instruction: &mut CodeInstruction, target: &str) -> Result<(), String> {
    for (key, value) in instruction.fields.iter_mut() {
        if *key == "target" {
            *value = Operand::Raw(target.into());
            return Ok(());
        }
    }
    Err(format!(
        "rv64 relax: {} has no target field",
        instruction.op.mnemonic()
    ))
}

/// A jump this round has to relax: where its `jal` sits, where it is going, and
/// the instruction index that carries it.
struct FarJump {
    index: usize,
    /// Byte offset of the `jal` word itself (not of the instruction).
    jal_at: isize,
    /// Byte offset of the target label.
    target_at: isize,
    target: String,
}

/// Relax one function's instruction list to a fixpoint. Splicing an island shifts
/// downstream offsets and can push a previously-in-range jump out of range, so
/// re-scan after each round until nothing is out of range.
///
/// Terminates: a round routes every far jump onto a ladder whose rungs are `HOP`
/// apart, so the round's own insertions (8 bytes per island) would have to consume
/// the whole `JAL_LIMIT - HOP` = 512 KiB slack — 65_536 islands in one function —
/// before a link failed to close. If a round nevertheless leaves a jump out of
/// range, the next one shortens it by a further `HOP`, so the worst remaining
/// displacement decreases monotonically; the round count is bounded so that a
/// pathological non-convergence is a diagnostic rather than a hang.
fn relax_function(
    instructions: &mut Vec<CodeInstruction>,
    function: &str,
    counter: &mut usize,
) -> Result<(), String> {
    // A round removes at least `HOP` bytes from the worst displacement, and the
    // worst displacement a function can hold is bounded by its own size; 64 rounds
    // covers a 32 MiB span of pure relaxation failure, far past anything a real
    // program produces. Exceeding it means the pass is not converging — report
    // that instead of looping.
    const MAX_ROUNDS: usize = 64;
    for _ in 0..MAX_ROUNDS {
        // One offset walk: the byte offset of each instruction and each label. A
        // `label` contributes 0 bytes (it only marks a position), so the uniform
        // `+= instruction_size` advance places it at the current offset.
        let mut offsets: Vec<isize> = Vec::with_capacity(instructions.len());
        let mut labels: HashMap<String, isize> = HashMap::new();
        let mut offset = 0isize;
        for instruction in instructions.iter() {
            offsets.push(offset);
            if instruction.op == CodeOp::Label {
                let name = instruction
                    .get("name")
                    .ok_or_else(|| "rv64 relax: label without a name".to_string())?;
                labels.insert(name, offset);
            }
            offset += instruction_size(instruction)? as isize;
        }

        // Collect every label-relative `jal` whose target sits outside imm20 reach.
        let mut far: Vec<FarJump> = Vec::new();
        for (index, instruction) in instructions.iter().enumerate() {
            let Some(within) = jal_offset_within(instruction.op) else {
                continue;
            };
            let target = instruction.get("target").ok_or_else(|| {
                format!("rv64 relax: {} without a target", instruction.op.mnemonic())
            })?;
            // An unresolved target is left for the encoder to diagnose (it owns the
            // "label does not resolve" error); relaxation only moves targets that
            // exist.
            let Some(&target_at) = labels.get(&target) else {
                continue;
            };
            let jal_at = offsets[index] + within as isize;
            let delta = target_at - jal_at;
            if delta < -JAL_LIMIT || delta >= JAL_LIMIT {
                far.push(FarJump {
                    index,
                    jal_at,
                    target_at,
                    target,
                });
            }
        }

        if far.is_empty() {
            return Ok(());
        }

        // Build the hop ladders this round needs. Every far jump to the same target
        // *from the same side* shares one ladder: rungs at `HOP` intervals from the
        // target out to the farthest source, each rung jumping to the next rung
        // closer in, and the innermost rung naming the real target. A jump then
        // needs a single retarget onto the rung nearest it. Sharing matters: a large
        // function reaches one trap stub from thousands of sites, and a private
        // chain per site would insert millions of islands.
        //
        // `ladders` is a `Vec` keyed by a side table rather than a `HashMap` of
        // ladders: the rung labels and the insertion order are derived from the
        // iteration order, and a `HashMap` deciding emission order is exactly the
        // shape that makes codegen non-deterministic. First-encounter order is a
        // function of the instruction stream, so it is stable.
        let mut ladder_of: HashMap<(String, bool), usize> = HashMap::new();
        let mut ladders: Vec<Ladder> = Vec::new();
        for jump in &far {
            let forward = jump.target_at > jump.jal_at;
            let index = *ladder_of
                .entry((jump.target.clone(), forward))
                .or_insert_with(|| {
                    ladders.push(Ladder {
                        target: jump.target.clone(),
                        target_at: jump.target_at,
                        forward,
                        span: 0,
                        rungs: Vec::new(),
                    });
                    ladders.len() - 1
                });
            let ladder = &mut ladders[index];
            ladder.span = ladder.span.max((jump.target_at - jump.jal_at).abs());
        }
        // Materialize each ladder's rungs, innermost (rung 1, one hop from the
        // target) first, so rung `k` can name rung `k - 1`.
        let mut inserts: Vec<(usize, Vec<CodeInstruction>)> = Vec::new();
        for ladder in ladders.iter_mut() {
            let count = (ladder.span / HOP) as usize;
            for k in 1..=count {
                let want = if ladder.forward {
                    ladder.target_at - (k as isize) * HOP
                } else {
                    ladder.target_at + (k as isize) * HOP
                };
                // First instruction at or after `want` — the rung lands within one
                // instruction's width of the intended offset, which the `HOP` slack
                // absorbs many times over.
                let at = offsets.partition_point(|&offset| offset < want);
                *counter += 1;
                let hop = format!("__mfb_rv_hop_{counter}");
                let over = format!("__mfb_rv_hop_over_{counter}");
                let onward = match ladder.rungs.last() {
                    None => ladder.target.clone(),
                    Some(inner) => inner.clone(),
                };
                inserts.push((at, island(&hop, &over, &onward)));
                ladder.rungs.push(hop);
            }
        }
        // Route every far jump onto the outermost rung that is still on its own side
        // of the target — `distance / HOP` rungs in, which is at most `HOP` (plus one
        // instruction's width) away.
        for jump in &far {
            let forward = jump.target_at > jump.jal_at;
            let ladder = &ladders[*ladder_of
                .get(&(jump.target.clone(), forward))
                .expect("every far jump registered its ladder")];
            let distance = (jump.target_at - jump.jal_at).abs();
            let index = ((distance / HOP) as usize).min(ladder.rungs.len());
            let label = ladder
                .rungs
                .get(index.saturating_sub(1))
                .ok_or_else(|| format!("rv64 relax: no hop rung for a far jump in '{function}'"))?
                .clone();
            retarget(&mut instructions[jump.index], &label)?;
        }

        if inserts.is_empty() {
            // Every ladder came out empty although a jump was out of range —
            // impossible, since a far jump is at least `2 * HOP` from its target.
            // Guard rather than spin.
            return Err(format!(
                "rv64 relax: no hop placed for {} out-of-range jump(s) in '{function}'",
                far.len()
            ));
        }
        // Apply every insertion in a single rebuild. Splicing one at a time is
        // O(len) per island, which is quadratic on exactly the huge functions this
        // pass exists for.
        inserts.sort_by_key(|(at, _)| *at);
        let inserted: usize = inserts.iter().map(|(_, body)| body.len()).sum();
        let mut rebuilt = Vec::with_capacity(instructions.len() + inserted);
        let mut pending = inserts.into_iter().peekable();
        for (index, instruction) in instructions.drain(..).enumerate() {
            while pending.peek().is_some_and(|(at, _)| *at == index) {
                rebuilt.extend(pending.next().expect("peeked").1);
            }
            rebuilt.push(instruction);
        }
        // A rung wanted past the last instruction lands at the end of the function.
        for (_, body) in pending {
            rebuilt.extend(body);
        }
        *instructions = rebuilt;
        // Loop: the inserted bytes may have pushed other jumps out of range.
    }
    Err(format!(
        "rv64 relax: branch relaxation did not converge in function '{function}'"
    ))
}

/// One hop ladder: the rungs that carry every far jump to a single target label
/// from a single side. Rungs are built innermost first, so `rungs[k - 1]` is the
/// rung `k` hops out from the target and names `rungs[k - 2]` (or the target
/// itself, for `k = 1`).
struct Ladder {
    target: String,
    target_at: isize,
    /// True when the target sits *after* its jumps, so the rungs run backwards from
    /// it; false when the jumps sit after the target.
    forward: bool,
    /// The farthest jump this ladder must reach, in bytes — it sets the rung count.
    span: isize,
    /// The rungs' hop labels, innermost first.
    rungs: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::engine::types::{CodeFrame, CodeFunction, NativeCodePlan};

    fn plan_of(instructions: Vec<CodeInstruction>) -> NativeCodePlan {
        let function = CodeFunction {
            name: "main".to_string(),
            symbol: "main".to_string(),
            params: Vec::new(),
            returns: "Nothing".to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations: Vec::new(),
            stack_slots: Vec::new(),
        };
        NativeCodePlan {
            target: "linux-riscv64".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            arch: "riscv64".to_string(),
            project: "t".to_string(),
            entry_symbol: Some("main".to_string()),
            imports: Vec::new(),
            data_objects: Vec::new(),
            functions: vec![function],
        }
    }

    /// `<branch> far`, then enough one-word `ret`s to put `far:` `span` bytes away.
    fn far_jump(branch: CodeInstruction, span: usize) -> Vec<CodeInstruction> {
        let words = span / 4;
        let mut instructions = Vec::with_capacity(words + 3);
        instructions.push(branch);
        for _ in 0..words {
            instructions.push(CodeInstruction::new("ret"));
        }
        instructions.push(CodeInstruction::new("label").field("name", "far"));
        instructions.push(CodeInstruction::new("ret"));
        instructions
    }

    fn branch_b() -> CodeInstruction {
        CodeInstruction::new("b").field("target", "far")
    }

    fn branch_rv_br() -> CodeInstruction {
        CodeInstruction::new("rv.br")
            .field("cond", "eq")
            .field("lhs", "a0")
            .field("rhs", "a1")
            .field("target", "far")
    }

    /// Decode a `jal`'s signed byte displacement out of its J-type immediate.
    fn jal_displacement(word: u32) -> i32 {
        assert_eq!(word & 0x7f, 0x6f, "not a jal: {word:#010x}");
        let imm20 = ((word >> 31) & 1) as i32;
        let imm10_1 = ((word >> 21) & 0x3ff) as i32;
        let imm11 = ((word >> 20) & 1) as i32;
        let imm19_12 = ((word >> 12) & 0xff) as i32;
        let value = (imm20 << 20) | (imm19_12 << 12) | (imm11 << 11) | (imm10_1 << 1);
        // Sign-extend from bit 20.
        (value << 11) >> 11
    }

    fn text_words(image: &crate::arch::riscv64::encode::EncodedImage) -> Vec<u32> {
        image
            .text
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }

    /// bug-453: the pre-fix behavior the relaxation removes — the encoder refuses
    /// the out-of-range `jal` rather than masking it to a wrong target. Kept as a
    /// pin: relaxation must be what makes the jump encode, never a widened bound.
    #[test]
    fn far_b_is_rejected_without_relaxation() {
        let plan = plan_of(far_jump(branch_b(), (JAL_LIMIT as usize) + 64));
        let err = match super::super::encode(&plan) {
            Ok(_) => panic!("expected an out-of-range rejection"),
            Err(err) => err,
        };
        assert!(
            err.contains("exceeds \u{00b1}1 MiB"),
            "expected an imm20 range error, got: {err}"
        );
    }

    /// bug-453 (the half the report missed): `rv.br`'s long form escapes the ±4 KiB
    /// B-type through a `jal`, so a conditional branch overflows at the very same
    /// threshold, through a different emitter path.
    #[test]
    fn far_rv_br_is_rejected_without_relaxation() {
        let plan = plan_of(far_jump(branch_rv_br(), (JAL_LIMIT as usize) + 64));
        let err = match super::super::encode(&plan) {
            Ok(_) => panic!("expected an out-of-range rejection"),
            Err(err) => err,
        };
        assert!(
            err.contains("exceeds \u{00b1}1 MiB"),
            "expected an imm20 range error, got: {err}"
        );
    }

    #[test]
    fn relaxation_makes_a_far_b_encode() {
        let mut plan = plan_of(far_jump(branch_b(), (JAL_LIMIT as usize) + 64));
        relax_rv64_branches(&mut plan).expect("relaxation");
        let image = super::super::encode(&plan).expect("encode after relaxation");
        let words = text_words(&image);
        // The branch is still one `jal zero` — relaxation rewrites its target, never
        // its size — and now hops forward by at most one HOP.
        let first = words[0];
        assert_eq!(first & 0x7f, 0x6f, "still a jal");
        assert_eq!(
            (first >> 7) & 0x1f,
            0,
            "still `jal zero` (no link register)"
        );
        let hop = jal_displacement(first);
        assert!(
            hop > 0 && (hop as isize) <= HOP + 16,
            "first hop should be a forward hop of at most one HOP, got {hop}"
        );
    }

    #[test]
    fn relaxation_makes_a_far_rv_br_encode() {
        let mut plan = plan_of(far_jump(branch_rv_br(), (JAL_LIMIT as usize) + 64));
        relax_rv64_branches(&mut plan).expect("relaxation");
        let image = super::super::encode(&plan).expect("encode after relaxation");
        let words = text_words(&image);
        // Word 0 is still the inverted 8-byte-skip B-type; word 1 is still the `jal`,
        // now aimed at the hop island.
        assert_eq!(words[0] & 0x7f, 0x63, "still a B-type short branch");
        assert_eq!(words[1] & 0x7f, 0x6f, "still a jal");
        let hop = jal_displacement(words[1]);
        assert!(
            hop > 0 && (hop as isize) <= HOP + 16,
            "first hop should be a forward hop of at most one HOP, got {hop}"
        );
    }

    /// A span wider than a single hop needs a *chain*: the branch reaches island 1,
    /// island 1 reaches island 2, and only the last island names `far`. Every `jal`
    /// in the encoded image must be in range — which `encode` already enforces — and
    /// following the chain from the branch must arrive at `far`.
    #[test]
    fn a_multi_hop_chain_walks_to_the_target() {
        let span = 5 * (JAL_LIMIT as usize) + 64; // 5 MiB: at least 10 hops at HOP = 512 KiB
        let mut plan = plan_of(far_jump(branch_b(), span));
        relax_rv64_branches(&mut plan).expect("relaxation");
        let image = super::super::encode(&plan).expect("encode after relaxation");
        let words = text_words(&image);
        // Walk the chain from word 0 and require it to terminate at the `far` label.
        // `far:` sits immediately before the final `ret`, i.e. at the last word.
        let far_word_index = words.len() - 1;
        let mut at = 0usize;
        let mut hops = 0usize;
        while at != far_word_index {
            let word = words[at];
            assert_eq!(word & 0x7f, 0x6f, "chain word {at} is not a jal");
            let delta = jal_displacement(word);
            assert!(
                (delta as isize) < JAL_LIMIT && (delta as isize) >= -JAL_LIMIT,
                "chain hop {hops} is out of imm20 range: {delta}"
            );
            at = (at as i64 + (delta as i64) / 4) as usize;
            hops += 1;
            assert!(hops < 64, "chain did not terminate after {hops} hops");
        }
        assert!(hops >= 10, "a 5 MiB span needs several hops, took {hops}");
    }

    /// The island is unreachable code in the middle of the stream, so the
    /// instruction that falls into it must jump over it. Encode a chain and check
    /// that every island opens with a `jal zero` skipping exactly its own two words.
    #[test]
    fn every_island_is_jumped_over() {
        let mut plan = plan_of(far_jump(branch_b(), 3 * (JAL_LIMIT as usize)));
        relax_rv64_branches(&mut plan).expect("relaxation");
        let names: Vec<String> = plan.functions[0]
            .instructions
            .iter()
            .filter(|i| i.op == CodeOp::Label)
            .filter_map(|i| i.get("name"))
            .collect();
        let hops = names
            .iter()
            .filter(|n| n.starts_with("__mfb_rv_hop_"))
            .count();
        let overs = names
            .iter()
            .filter(|n| n.starts_with("__mfb_rv_hop_over_"))
            .count();
        // `starts_with("__mfb_rv_hop_")` also matches the `_over_` names.
        assert_eq!(hops - overs, overs, "each hop label needs one continuation");
        let image = super::super::encode(&plan).expect("encode after relaxation");
        let words = text_words(&image);
        // Each island is `jal zero, over` / `jal zero, next`; the first must skip two
        // words (8 bytes) to land on the continuation.
        let mut islands = 0usize;
        for (index, &word) in words.iter().enumerate() {
            if word & 0x7f == 0x6f && jal_displacement(word) == 8 && index > 0 {
                islands += 1;
            }
        }
        assert!(islands >= 5, "expected an island per hop, found {islands}");
    }

    /// Two far jumps to two different targets in one function: the rung labels and
    /// the island order are derived from iteration order, so relaxing the same plan
    /// twice must produce byte-identical text. A `HashMap` deciding emission order
    /// would read as a flaky golden rather than as this test.
    #[test]
    fn relaxation_is_deterministic_across_two_targets() {
        fn two_target_plan() -> NativeCodePlan {
            let span = 2 * (JAL_LIMIT as usize);
            let mut instructions = vec![
                CodeInstruction::new("b").field("target", "far"),
                CodeInstruction::new("b").field("target", "other"),
            ];
            for _ in 0..(span / 4) {
                instructions.push(CodeInstruction::new("ret"));
            }
            instructions.push(CodeInstruction::new("label").field("name", "far"));
            for _ in 0..(span / 4) {
                instructions.push(CodeInstruction::new("ret"));
            }
            instructions.push(CodeInstruction::new("label").field("name", "other"));
            instructions.push(CodeInstruction::new("ret"));
            plan_of(instructions)
        }
        let mut first = two_target_plan();
        relax_rv64_branches(&mut first).expect("relaxation");
        let first = super::super::encode(&first).expect("encode").text;
        let mut second = two_target_plan();
        relax_rv64_branches(&mut second).expect("relaxation");
        let second = super::super::encode(&second).expect("encode").text;
        assert_eq!(first, second, "relaxation must be deterministic");
    }

    /// The pass is a strict no-op when nothing is out of range: the instruction list
    /// comes out with the same length and the same ops, so every existing
    /// `linux-riscv64` golden is byte-identical.
    #[test]
    fn in_range_jumps_are_left_untouched() {
        let instructions = vec![
            branch_b(),
            CodeInstruction::new("ret"),
            branch_rv_br(),
            CodeInstruction::new("label").field("name", "far"),
            CodeInstruction::new("ret"),
        ];
        let before = super::super::encode(&plan_of(instructions.clone()))
            .expect("encode")
            .text;
        let mut plan = plan_of(instructions);
        relax_rv64_branches(&mut plan).expect("relaxation");
        assert_eq!(
            plan.functions[0].instructions.len(),
            5,
            "no island inserted"
        );
        let after = super::super::encode(&plan).expect("encode").text;
        assert_eq!(before, after, "in-range code must be byte-identical");
    }

    /// An unresolved target is the encoder's diagnostic to own, not this pass's: it
    /// must be left alone rather than relaxed into a chain aimed at nothing.
    #[test]
    fn an_unresolved_target_is_left_to_the_encoder() {
        let mut plan = plan_of(vec![
            CodeInstruction::new("b").field("target", "nowhere"),
            CodeInstruction::new("ret"),
        ]);
        relax_rv64_branches(&mut plan).expect("relaxation leaves it alone");
        assert_eq!(plan.functions[0].instructions.len(), 2);
        let err = match super::super::encode(&plan) {
            Ok(_) => panic!("expected the encoder's unresolved-label error"),
            Err(err) => err,
        };
        assert!(
            err.contains("does not resolve"),
            "expected the encoder's unresolved-label error, got: {err}"
        );
    }
}
