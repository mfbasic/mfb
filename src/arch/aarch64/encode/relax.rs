//! bug-445: AArch64 conditional-branch relaxation (veneer insertion).
//!
//! AArch64 `B.<cond>` encodes its target as a 19-bit signed word offset
//! (`imm19`), reaching only ±1 MiB. The shared two-pass encoder
//! ([`crate::arch::encode_plan`]) validates that reach in
//! [`super::emitter::Encoder::patch_labels`] and — correctly, since bug-124 —
//! refuses to mask an out-of-range displacement to a wrong target. But refusing
//! is not the only option: a large function whose conditional branch must span
//! more than ±1 MiB is *legal* and should compile. The standard fix is branch
//! relaxation — rewrite the far conditional into a short conditional that hops to
//! a nearby trampoline holding an unconditional `B` (which reaches ±128 MiB via
//! `imm26`). GCC/LLVM/ld all do this; this pass gives the AArch64 backend the
//! same.
//!
//! The rewrite keeps the *original* condition rather than inverting it, so it
//! needs no new opcode (the backend emits `b.lo`/`b.mi` but not their
//! complements `b.hs`/`b.pl`):
//!
//! ```text
//!     b.<cond> far                b.<cond> Ltramp   ; near: taken -> trampoline
//!                        ==>       b        Lcont    ; not taken -> fall through
//!                                Ltramp:
//!                                  b        far      ; unconditional, ±128 MiB
//!                                Lcont:
//! ```
//!
//! It runs on the plan's instruction stream *before* encoding, so the normal
//! two-pass encoder then sizes and places the enlarged stream naturally and every
//! in-range branch is byte-identical to before (the pass is a strict no-op when
//! no conditional branch is out of range — the case for every realistic program).

use super::sizing::instruction_size;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::{CodeInstruction, NativeCodePlan};
use std::collections::HashMap;

/// `imm19` reach in bytes: ±2^18 words × 4 = ±1 MiB. Mirrors the bound
/// [`super::emitter::Encoder::patch_labels`] enforces, so a branch this pass
/// leaves alone is exactly one the encoder accepts.
const IMM19_LIMIT: isize = 1 << 20;

/// The conditional branch kinds the AArch64 backend emits (`emitter.rs`). Each
/// is one `imm19`-reach instruction; an unconditional `Branch` (`imm26`) and the
/// `bl`/`blr` calls are excluded — they reach far enough or are not PC-relative
/// label branches.
fn is_conditional_branch(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::BranchEq
            | CodeOp::BranchNe
            | CodeOp::BranchGe
            | CodeOp::BranchLt
            | CodeOp::BranchGt
            | CodeOp::BranchLe
            | CodeOp::BranchVc
            | CodeOp::BranchVs
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::BranchMi
            | CodeOp::BranchLs
    )
}

/// Rewrite every out-of-`imm19`-range conditional branch in the plan into a
/// short-hop-to-trampoline sequence so the whole plan encodes. A no-op (leaves
/// the instruction stream byte-for-byte unchanged) when every conditional branch
/// already fits — the case for every program that compiled before bug-445.
pub(crate) fn relax_conditional_branches(plan: &mut NativeCodePlan) -> Result<(), String> {
    // `-vv` (`crate::trace`): the pass rewrites nothing for a normal program but
    // still relaxes each function to a fixpoint, so its cost is a scan of every
    // instruction in the plan — worth a row of its own rather than hiding in the
    // "emitting native code" stage's self time.
    let _span = crate::trace::span("relax branches");
    // A single monotonic counter across the whole plan keeps every synthesized
    // trampoline/continuation label globally unique (labels are function-local,
    // but a shared counter is simplest and still unique).
    let mut counter = 0usize;
    for function in &mut plan.functions {
        relax_function(&mut function.instructions, &mut counter)?;
    }
    Ok(())
}

/// Relax one function's instruction list to a fixpoint. Inserting a veneer shifts
/// downstream offsets and can push a previously-in-range branch out of range, so
/// re-scan after each rewriting pass until no conditional branch is out of range.
/// Terminates: a rewritten branch's new conditional hop targets a trampoline two
/// instructions away (always in range), so it is never rewritten again; the set
/// of relaxed branches only grows and is bounded by the branch count.
fn relax_function(
    instructions: &mut Vec<CodeInstruction>,
    counter: &mut usize,
) -> Result<(), String> {
    loop {
        // One offset walk: record the byte offset of each instruction and each
        // label. A `label` contributes 0 bytes (it only marks a position), so the
        // uniform `+= instruction_size` advance places it at the current offset.
        let mut offsets: Vec<usize> = Vec::with_capacity(instructions.len());
        let mut labels: HashMap<String, usize> = HashMap::new();
        let mut offset = 0usize;
        for instruction in instructions.iter() {
            offsets.push(offset);
            if instruction.op == CodeOp::Label {
                let name = instruction
                    .get("name")
                    .ok_or_else(|| "AArch64 relax: label without a name".to_string())?;
                labels.insert(name, offset);
            }
            offset += instruction_size(instruction)?;
        }

        // Collect every conditional branch whose target sits outside imm19 reach.
        let mut out_of_range: Vec<usize> = Vec::new();
        for (index, instruction) in instructions.iter().enumerate() {
            if !is_conditional_branch(instruction.op) {
                continue;
            }
            let target = instruction
                .get("target")
                .ok_or_else(|| "AArch64 relax: conditional branch without a target".to_string())?;
            // An unresolved target is left for the encoder to diagnose (it owns
            // the "label does not resolve" error); relaxation only moves targets
            // that exist.
            let Some(&target_offset) = labels.get(&target) else {
                continue;
            };
            let delta = target_offset as isize - offsets[index] as isize;
            if delta < -IMM19_LIMIT || delta >= IMM19_LIMIT {
                out_of_range.push(index);
            }
        }

        if out_of_range.is_empty() {
            return Ok(());
        }

        // Rewrite back-to-front so each splice leaves the still-to-process
        // (smaller) indices valid.
        for &index in out_of_range.iter().rev() {
            let condition = instructions[index].op.mnemonic().to_string();
            let far_target = instructions[index]
                .get("target")
                .expect("checked above that the branch has a target");
            *counter += 1;
            let trampoline = format!("__mfb_relax_tramp_{counter}");
            let continuation = format!("__mfb_relax_cont_{counter}");
            let veneer = vec![
                // Taken: hop to the nearby trampoline (well within imm19).
                CodeInstruction::new(&condition).field("target", trampoline.clone()),
                // Not taken: skip past the trampoline.
                CodeInstruction::new("b").field("target", continuation.clone()),
                CodeInstruction::new("label").field("name", trampoline),
                // The far jump, now unconditional (imm26, ±128 MiB).
                CodeInstruction::new("b").field("target", far_target),
                CodeInstruction::new("label").field("name", continuation),
            ];
            instructions.splice(index..index + 1, veneer);
        }
        // Loop: the inserted bytes may have pushed other branches out of range.
    }
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
            target: "linux-aarch64".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            arch: "aarch64".to_string(),
            project: "t".to_string(),
            entry_symbol: Some("main".to_string()),
            imports: Vec::new(),
            data_objects: Vec::new(),
            functions: vec![function],
        }
    }

    /// A `b.eq` whose target label sits just past ±1 MiB: `b.eq far`, then enough
    /// one-word `ret`s to overflow imm19, then `far:`. Padding count is chosen so
    /// the displacement crosses 2^20 bytes (2^18 words) with margin.
    fn far_conditional_branch() -> Vec<CodeInstruction> {
        let words = (IMM19_LIMIT as usize / 4) + 16;
        let mut instructions = Vec::with_capacity(words + 3);
        instructions.push(CodeInstruction::new("b.eq").field("target", "far"));
        for _ in 0..words {
            instructions.push(CodeInstruction::new("ret"));
        }
        instructions.push(CodeInstruction::new("label").field("name", "far"));
        instructions.push(CodeInstruction::new("ret"));
        instructions
    }

    #[test]
    fn far_conditional_branch_is_rejected_without_relaxation() {
        // bug-445: this is the pre-fix behavior the relaxation pass removes — the
        // encoder refuses the out-of-range conditional rather than masking it.
        let plan = plan_of(far_conditional_branch());
        let err = match super::super::encode(&plan) {
            Ok(_) => panic!("expected out-of-range rejection"),
            Err(err) => err,
        };
        assert!(
            err.contains("exceeds \u{00b1}1 MiB"),
            "expected an imm19 range error, got: {err}"
        );
    }

    #[test]
    fn relaxation_makes_a_far_conditional_branch_encode() {
        let mut plan = plan_of(far_conditional_branch());
        relax_conditional_branches(&mut plan).expect("relaxation");
        // The far conditional is now a short hop to a trampoline; the whole plan
        // encodes instead of erroring.
        let image = match super::super::encode(&plan) {
            Ok(image) => image,
            Err(err) => panic!("encode after relaxation failed: {err}"),
        };

        // The first instruction is still a `b.eq`, but now to the trampoline eight
        // bytes ahead (imm19 word offset = 2): 0x5400_0000 | (2 << 5) = 0x5400_0040.
        let first = u32::from_le_bytes(image.text[..4].try_into().unwrap());
        assert_eq!(
            first, 0x5400_0040,
            "relaxed conditional should hop +8 to the trampoline"
        );
        // The second word is an unconditional `b` skipping over the trampoline's
        // single `b` (to the continuation two words ahead): 0x1400_0000 | 2.
        let second = u32::from_le_bytes(image.text[4..8].try_into().unwrap());
        assert_eq!(second, 0x1400_0002, "not-taken path skips the trampoline");
        // The trampoline's unconditional `b` (third word) carries the full far
        // displacement in imm26 and must be in range (top opcode bits `000101`).
        let third = u32::from_le_bytes(image.text[8..12].try_into().unwrap());
        assert_eq!(third >> 26, 0b000101, "trampoline holds an unconditional b");
    }

    #[test]
    fn in_range_conditional_branch_is_left_untouched() {
        // A short forward `b.eq` must survive relaxation byte-for-byte: the pass is
        // a strict no-op when nothing is out of range.
        let instructions = vec![
            CodeInstruction::new("b.eq").field("target", "near"),
            CodeInstruction::new("ret"),
            CodeInstruction::new("label").field("name", "near"),
            CodeInstruction::new("ret"),
        ];
        let mut plan = plan_of(instructions);
        relax_conditional_branches(&mut plan).expect("relaxation");
        // Exactly the original four instructions remain (no veneer inserted).
        assert_eq!(plan.functions[0].instructions.len(), 4);
        assert_eq!(plan.functions[0].instructions[0].op, CodeOp::BranchEq);
    }
}
