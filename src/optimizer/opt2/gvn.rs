//! Global value numbering (GVN) — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): whole-function redundancy elimination on
//! the SSA overlay ([`super::plans::ssa`]). An instruction recomputing an
//! expression — same operation, operands resolving to the same SSA *values*
//! — already computed by a **dominating** instruction is rewritten to a copy
//! of that instruction's result; copy propagation bypasses the copy and DCE
//! sweeps the strands. This also lands the "Common subexpression elimination
//! (CSE)" row, whose own catalog description declares it subsumed by GVN on
//! SSA (the block-local complement is `opt2::lvn`).
//!
//! **Dominance is the whole availability argument** — and it covers the
//! *value*, not the register. If the candidate's block dominates the reuse
//! and any operand value had been redefined between the candidate's last
//! execution and the reuse, a path to the reuse avoiding the candidate would
//! have to exist (the operand's definition cannot be dominated *by* the
//! candidate while also dominating it), contradicting dominance; so the
//! candidate's computed value is always current at the reuse. Whether the
//! candidate's *register* still holds it is separate: a single-def dst
//! provably does (no redefinition exists anywhere) and is read directly; a
//! multi-def dst may be stale, so the value is parked in a **fresh
//! single-def holder** — a minted vreg copied from the dst immediately after
//! the candidate — and the reuse reads that. Both checks are independent of
//! phi placement (unlike copy forwarding's stack check).
//!
//! Expression keys are SSA value ids **canonicalized** through copy chains
//! and `mov_imm` (Integer) feeders — two different `mov_imm 8`s are distinct
//! SSA values but the same constant, and without folding them to one key
//! part essentially no constant-fed expression ever matches (measured: zero
//! table hits across the example corpus). Commutative operations sort their
//! operands. Everything runs in virtual-register space (a physical operand
//! is ineligible — hidden clobbers), operations are the pure flag-free
//! whitelist minus the moves, so checked arithmetic is never touched and
//! every rewrite copies bits-identical values. Arm-and-arm redundancy
//! meeting at a join (neither arm dominates) is partial-redundancy
//! elimination — the separate PRE row, not this one.

use std::collections::HashMap;

use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::{
    build_cfg, classify_ref, effect, ClassModel, RegRef,
};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::lvn::{commutative, copy_of, key_rank, numberable_op, KeyPart};
use super::plans::ssa::{self, Ssa, ValueDef, Var};

/// A reusable earlier computation.
struct Candidate {
    inst: usize,
    block: usize,
    dst_var: Var,
    dst_operand: Operand,
    /// The fresh single-def holder minted for this candidate, once some
    /// reuse needed one (the candidate's own dst has other definitions).
    fresh: Option<Operand>,
}

/// Run the GVN row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (3).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        crate::optimizer::stats::count_global_value_numberings(0);
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);

    // Which block each instruction lives in.
    let mut block_of = vec![0usize; instructions.len()];
    for (index, block) in blocks.iter().enumerate() {
        for slot in &mut block_of[block.start..block.end] {
            *slot = index;
        }
    }
    // Definition counts per int variable (single-def holders reuse directly),
    // and the vreg-id ceiling for minting fresh holders.
    let mut def_count: HashMap<Var, u32> = HashMap::new();
    let mut max_vreg: u32 = 0;
    for instruction in instructions.iter() {
        for class_model in [&models.0, &models.1] {
            let eff = effect(instruction, class_model);
            for def in eff.defs.iter().chain(eff.uses.iter()) {
                if let RegRef::VReg(id) = def {
                    if !class_model.is_fp {
                        max_vreg = max_vreg.max(*id);
                    }
                }
            }
            for def in eff.defs {
                if let RegRef::VReg(id) = def {
                    *def_count.entry((class_model.is_fp, id)).or_insert(0) += 1;
                }
            }
        }
    }
    let mut next_vreg = max_vreg + 1;

    let mut table: HashMap<(crate::arch::ops::CodeOp, Vec<KeyPart>), Vec<Candidate>> =
        HashMap::new();
    // `mov <fresh>, <candidate dst>` copies to splice in *after* these
    // (old-stream) indices once the scan is done.
    let mut insertions: Vec<(usize, CodeInstruction)> = Vec::new();
    let mut fired = 0;
    for i in 0..instructions.len() {
        if !numberable_op(instructions[i].op) {
            continue;
        }
        let Some((key, dst_var, dst_operand)) = value_key(instructions, i, &models, &overlay)
        else {
            continue;
        };
        let mut replacement: Option<CodeInstruction> = None;
        if let Some(candidates) = table.get_mut(&key) {
            for candidate in candidates.iter_mut() {
                let executes_first = candidate.block == block_of[i] && candidate.inst < i
                    || candidate.block != block_of[i]
                        && overlay.dominates(candidate.block, block_of[i]);
                if !executes_first {
                    continue;
                }
                // Dominance is the whole availability argument: were any
                // operand value redefined between the candidate's last
                // execution and this point, a path to here avoiding the
                // candidate would exist — contradicting dominance. The value
                // is therefore current; only the *register* may have moved
                // on. A single-def dst is reused directly; otherwise the
                // value is parked in a fresh single-def holder right after
                // the candidate.
                let source = if def_count.get(&candidate.dst_var) == Some(&1) {
                    candidate.dst_operand.clone()
                } else {
                    match &candidate.fresh {
                        Some(fresh) => fresh.clone(),
                        None => {
                            let fresh = Operand::vreg(
                                crate::target::shared::regmodel::RegClass::Int,
                                next_vreg,
                            );
                            next_vreg += 1;
                            insertions.push((
                                candidate.inst,
                                copy_of(fresh.clone(), candidate.dst_operand.clone()),
                            ));
                            candidate.fresh = Some(fresh.clone());
                            fresh
                        }
                    }
                };
                replacement = Some(copy_of(dst_operand.clone(), source));
                break;
            }
        }
        match replacement {
            Some(copy) => {
                instructions[i] = copy;
                fired += 1;
            }
            None => {
                table.entry(key).or_default().push(Candidate {
                    inst: i,
                    block: block_of[i],
                    dst_var,
                    dst_operand,
                    fresh: None,
                });
            }
        }
    }
    if !insertions.is_empty() {
        insertions.sort_by_key(|(after, _)| *after);
        let mut merged = Vec::with_capacity(instructions.len() + insertions.len());
        let mut pending = insertions.into_iter().peekable();
        for (index, instruction) in std::mem::take(instructions).into_iter().enumerate() {
            merged.push(instruction);
            while pending.peek().is_some_and(|(after, _)| *after == index) {
                merged.push(pending.next().expect("peeked").1);
            }
        }
        *instructions = merged;
    }
    crate::optimizer::stats::count_global_value_numberings(fired);
}

/// The instruction's SSA-value expression key plus its dst, when every
/// operand is an int vreg with a resolved SSA value or a non-register
/// literal, and the dst is a single int vreg.
fn value_key(
    instructions: &[CodeInstruction],
    inst: usize,
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
) -> Option<((crate::arch::ops::CodeOp, Vec<KeyPart>), Var, Operand)> {
    let instruction = &instructions[inst];
    let int_model = &models.0;
    let mut parts: Vec<KeyPart> = Vec::new();
    let mut dst: Option<(Var, Operand)> = None;
    for (name, operand) in &instruction.fields {
        if *name == "dst" {
            match classify_ref(operand, int_model) {
                Some(RegRef::VReg(id)) => dst = Some(((false, id), operand.clone())),
                _ => return None,
            }
            continue;
        }
        match classify_ref(operand, int_model) {
            Some(RegRef::VReg(id)) => {
                // Unresolved (an unreachable block): no facts, no transform.
                let value = overlay.value_of_use(inst, (false, id))?;
                parts.push(canonical(value, instructions, models, overlay));
            }
            Some(RegRef::Phys(_)) => return None,
            None => {
                if classify_ref(operand, &models.1).is_some() {
                    return None;
                }
                parts.push(KeyPart::Literal(operand.rendered().into_owned()));
            }
        }
    }
    let (dst_var, dst_operand) = dst?;
    if commutative(instruction.op) {
        parts.sort_by(|a, b| key_rank(a).cmp(&key_rank(b)));
    }
    Some(((instruction.op, parts), dst_var, dst_operand))
}

/// Canonicalize an operand's SSA value for keying: chase copy chains to the
/// underlying value, and fold a value defined by `mov_imm` (Integer) to its
/// constant bits — two different `mov_imm 8` feeders are *different* SSA
/// values but the same constant, and without this no constant-fed expression
/// ever matches another (measured: zero table hits across the whole example
/// corpus before this canonicalization).
fn canonical(
    mut value: usize,
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
) -> KeyPart {
    use crate::arch::ops::CodeOp;
    for _ in 0..64 {
        let ValueDef::Inst(d) = overlay.values[value] else {
            break;
        };
        let instruction = &instructions[d];
        match instruction.op {
            CodeOp::Mov => {
                let step =
                    instruction
                        .operand("src")
                        .and_then(|src| match classify_ref(src, &models.0) {
                            Some(RegRef::VReg(id)) => overlay.value_of_use(d, (false, id)),
                            _ => None,
                        });
                match step {
                    Some(next) => value = next,
                    None => break,
                }
            }
            CodeOp::MovImm => {
                if instruction.get("type").as_deref() == Some("Integer") {
                    if let Some(bits) = instruction.get("value").and_then(|text| bits_of(&text)) {
                        return KeyPart::Constant(bits);
                    }
                }
                break;
            }
            _ => break,
        }
    }
    KeyPart::Value(value as u64)
}

/// A literal field's 64-bit pattern (the folder's spelling rules).
fn bits_of(text: &str) -> Option<u64> {
    text.parse::<u64>()
        .ok()
        .or_else(|| text.parse::<i64>().ok().map(|signed| signed as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    use crate::arch::ops::CodeOp;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
        with_opt_level(OptLevel(level), || eliminate(stream, &Aarch64RegisterModel));
    }

    /// A recompute in a *dominated* block (past a join label LVN must forget
    /// at) reuses the dominating computation — the global in GVN.
    #[test]
    fn dominated_recompute_reuses_across_blocks() {
        let mut stream = vec![
            /* 0 */ ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            /* 1 */ ci("b.eq", &[("target", "join")]),
            /* 2 */
            ci(
                "str_u64",
                &[("src", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            /* 3 */ ci("label", &[("name", "join")]),
            /* 4 */ ci("add", &[("dst", "%v4"), ("lhs", "%v2"), ("rhs", "%v1")]),
            /* 5 */
            ci(
                "str_u64",
                &[("src", "%v4"), ("base", "sp"), ("offset", "16")],
            ),
            /* 6 */ ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[4].op, CodeOp::Mov, "block 0 dominates the join");
        assert_eq!(stream[4].get("src").as_deref(), Some("%v3"));
    }

    /// Arm-and-arm redundancy at a join is PRE, not GVN: neither arm
    /// dominates the join, so the recompute stays.
    #[test]
    fn non_dominating_arms_do_not_feed_the_join() {
        let mut stream = vec![
            ci("b.eq", &[("target", "else")]),
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "else")]),
            ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("label", &[("name", "join")]),
            ci("add", &[("dst", "%v5"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[6].op, CodeOp::Add, "no dominating candidate");
        // The second arm is not dominated by the first arm either.
        assert_eq!(stream[4].op, CodeOp::Add);
    }

    /// A holder with more than one definition is not read directly — the
    /// register may be stale at the reuse — but the *value* is provably
    /// current (dominance), so it is parked in a fresh single-def holder
    /// right after the candidate and the reuse reads that.
    #[test]
    fn multi_def_holders_reuse_through_a_fresh_holder() {
        let mut stream = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("label", &[("name", "next")]),
            ci(
                "ldr_u64",
                &[("dst", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream.len(), 6, "one fresh-holder copy inserted");
        assert_eq!(stream[1].op, CodeOp::Mov, "parked right after the add");
        assert_eq!(stream[1].get("dst").as_deref(), Some("%v5"));
        assert_eq!(stream[1].get("src").as_deref(), Some("%v3"));
        assert_eq!(
            stream[4].op,
            CodeOp::Mov,
            "the reuse reads the fresh holder"
        );
        assert_eq!(stream[4].get("src").as_deref(), Some("%v5"));
        assert_eq!(stream[4].get("dst").as_deref(), Some("%v4"));
    }

    /// Equal constants from *different* `mov_imm` feeders canonicalize to the
    /// same key part, so constant-fed expressions match across blocks.
    #[test]
    fn constant_feeders_canonicalize() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "8")],
            ),
            ci("add", &[("dst", "%v3"), ("lhs", "%v9"), ("rhs", "%v1")]),
            ci("label", &[("name", "next")]),
            ci(
                "mov_imm",
                &[("dst", "%v2"), ("type", "Integer"), ("value", "8")],
            ),
            ci("add", &[("dst", "%v4"), ("lhs", "%v9"), ("rhs", "%v2")]),
            ci(
                "str_u64",
                &[("src", "%v4"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[4].op, CodeOp::Mov, "same value: %v9 + 8");
        assert_eq!(stream[4].get("src").as_deref(), Some("%v3"));
    }

    /// Operand *values*, not names: a recompute whose inputs were redefined
    /// to new values does not match, even with identical spelling.
    #[test]
    fn value_identity_not_name_identity() {
        let mut stream = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("label", &[("name", "next")]),
            ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[3].op, CodeOp::Add, "%v1 holds a different value");
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = vec![
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("label", &[("name", "next")]),
            ci("add", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::Add);
    }
}
