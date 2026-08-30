//! Branch simplification / folding — the Opt2 (CFG-level) half of the
//! Level-2 catalog row (`planning/optimizations.md`): fold a conditional
//! branch whose outcome is decided by a compare of known constants. The tree
//! half lives in `opt1::branches`; both feed one "Branch simplification /
//! folding" `-v` count.
//!
//! Block-local and register-tracked, in the exact mold of the Opt2 constant
//! folder — the known-constant map is maintained through the very same
//! [`constant_folding::fold_one`] steps (shared function, no drift), and one
//! extra fact rides along: the operands of the last `cmp`/`cmp_imm` when both
//! are known ("the flags are `cmp a, b` with a and b known"). A conditional
//! flag branch reached with that fact folds: known-taken becomes an
//! unconditional `b target`, known-not-taken is deleted (fallthrough). The
//! newly unreachable code behind the fold is the UCE row's food — this is the
//! "folding `IF FALSE` is what *creates* statically-dead branches" pairing
//! the catalog's definition note names.
//!
//! **Trap preservation is structural, same as the folder:** flags are trusted
//! only when they come from `cmp`/`cmp_imm` — MFB's checked arithmetic sets
//! flags with `adds`/`subs`, which `fold_one` refuses to model (`Barrier`,
//! clearing both the constants *and* the flag fact), so an `ErrOverflow`
//! guard (`b.vs`/`b.vc`) is never folded (their evaluations are additionally
//! unimplemented, a belt over the braces). Every unmodeled instruction
//! likewise clears both facts; a label is a join; a conditional branch's own
//! fallthrough keeps the facts (falling through changes neither registers nor
//! flags). Mis-modeling can only lose a fold, never invent one.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;

use super::constant_folding::{fold_one, Step};
use super::plans::mark::{conditional_terminator, removable_op};

/// Run the Opt2 branch-folding row over one function's selected stream, in
/// place. Self-guarded on the row's catalog level (2).
pub(crate) fn fold_branches(instructions: &mut Vec<CodeInstruction>) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let mut known: HashMap<String, u64> = HashMap::new();
    // The last compare's (lhs, rhs) values, when both were known.
    let mut flags: Option<(u64, u64)> = None;
    // index -> Some(target) = rewrite to `b target`; None = delete.
    let mut folds: Vec<(usize, Option<CodeInstruction>)> = Vec::new();
    for (i, instruction) in instructions.iter().enumerate() {
        if conditional_terminator(instruction.op) {
            if let Some((a, b)) = flags {
                if let Some(taken) = verdict(instruction.op, a, b) {
                    folds.push((
                        i,
                        taken.then(|| {
                            let target = instruction
                                .operand("target")
                                .cloned()
                                .expect("conditional branches carry a target");
                            CodeInstruction::new("b").field("target", target)
                        }),
                    ));
                }
            }
            // The fallthrough continuation sees the same registers and flags.
            continue;
        }
        match instruction.op {
            CodeOp::Cmp => {
                let value = |field: &str| {
                    instruction
                        .get(field)
                        .and_then(|name| known.get(&name).copied())
                };
                flags = match (value("lhs"), value("rhs")) {
                    (Some(a), Some(b)) => Some((a, b)),
                    _ => None,
                };
            }
            CodeOp::CmpImm => {
                let lhs = instruction
                    .get("lhs")
                    .and_then(|name| known.get(&name).copied());
                let rhs = instruction.get("rhs").and_then(|text| bits(&text));
                flags = match (lhs, rhs) {
                    (Some(a), Some(b)) => Some((a, b)),
                    _ => None,
                };
            }
            op => {
                // The known map advances through the folder's own semantics;
                // the flag fact survives only the provably flag-free ops.
                let step = fold_one(instruction, &|field| {
                    instruction
                        .get(field)
                        .and_then(|name| known.get(&name).copied())
                });
                match step {
                    Step::Record(dst, value) | Step::Replace(dst, value) => {
                        known.insert(dst, value);
                    }
                    Step::KillDst => {
                        if let Some(dst) = instruction.get("dst") {
                            known.remove(&dst);
                        } else {
                            known.clear();
                        }
                    }
                    Step::Barrier => {
                        known.clear();
                        flags = None;
                    }
                }
                if !removable_op(op) {
                    flags = None;
                }
            }
        }
    }

    let folded = folds.len() as u64;
    if folded != 0 {
        let mut keep = vec![true; instructions.len()];
        for (index, replacement) in folds {
            match replacement {
                Some(branch) => instructions[index] = branch,
                None => keep[index] = false,
            }
        }
        let mut index = 0;
        instructions.retain(|_| {
            let keep = keep[index];
            index += 1;
            keep
        });
    }
    crate::optimizer::stats::count_branch_simplifications(folded);
}

/// Whether the conditional branch is taken after `cmp a, b`, when this pass
/// models the condition. `b.vs`/`b.vc` (the checked-arithmetic guards) and
/// the x86/riscv-specific branch families are deliberately unmodeled.
/// pub(super): SCCP decides edge reachability with the identical rules, so
/// the two rows cannot disagree about what a compare proves.
pub(super) fn verdict(op: CodeOp, a: u64, b: u64) -> Option<bool> {
    let (sa, sb) = (a as i64, b as i64);
    Some(match op {
        CodeOp::BranchEq => a == b,
        CodeOp::BranchNe => a != b,
        CodeOp::BranchGe => sa >= sb,
        CodeOp::BranchLt => sa < sb,
        CodeOp::BranchGt => sa > sb,
        CodeOp::BranchLe => sa <= sb,
        CodeOp::BranchHi => a > b,
        CodeOp::BranchLo => a < b,
        CodeOp::BranchLs => a <= b,
        // N flag of the subtraction — the sign of the wrapped difference,
        // which is not the same as signed `<` under overflow.
        CodeOp::BranchMi => (a.wrapping_sub(b) as i64) < 0,
        _ => return None,
    })
}

/// A literal field's 64-bit pattern (the folder's spelling rules).
pub(super) fn bits(text: &str) -> Option<u64> {
    text.parse::<u64>()
        .ok()
        .or_else(|| text.parse::<i64>().ok().map(|signed| signed as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
        ci(
            "mov_imm",
            &[("dst", dst), ("type", "Integer"), ("value", value)],
        )
    }

    fn ops(instructions: &[CodeInstruction]) -> Vec<CodeOp> {
        instructions.iter().map(|inst| inst.op).collect()
    }

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
        with_opt_level(OptLevel(level), || fold_branches(stream));
    }

    /// A known-taken compare-and-branch becomes an unconditional `b`; the
    /// (now statically dead) fallthrough is left for UCE.
    #[test]
    fn known_taken_branch_becomes_unconditional() {
        let mut stream = vec![
            mov_imm("%v1", "3"),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "3")]),
            ci("b.eq", &[("target", "yes")]),
            ci("bl", &[("target", "_never")]),
            ci("label", &[("name", "yes")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::Branch, "b.eq folds to b");
        assert_eq!(stream[2].get("target").as_deref(), Some("yes"));
    }

    /// A known-not-taken branch is deleted: control provably falls through.
    #[test]
    fn known_not_taken_branch_is_deleted() {
        let mut stream = vec![
            mov_imm("%v1", "3"),
            mov_imm("%v2", "4"),
            ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b.eq", &[("target", "skip")]),
            ci("label", &[("name", "skip")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert!(
            !ops(&stream).contains(&CodeOp::BranchEq),
            "3 == 4 is false: the b.eq goes"
        );
    }

    /// Signed vs unsigned conditions read the same bits differently.
    #[test]
    fn signed_and_unsigned_conditions_diverge() {
        // -1 vs 1: signed less-than is true, unsigned less-than is false.
        let stream = |branch: &str| {
            vec![
                mov_imm("%v1", "-1"),
                mov_imm("%v2", "1"),
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci(branch, &[("target", "t")]),
                ci("label", &[("name", "t")]),
                ci("ret", &[]),
            ]
        };
        let mut signed = stream("b.lt");
        run(&mut signed, 2);
        assert_eq!(signed[3].op, CodeOp::Branch, "signed -1 < 1 is taken");

        let mut unsigned = stream("b.lo");
        run(&mut unsigned, 2);
        assert!(
            !ops(&unsigned).contains(&CodeOp::BranchLo),
            "unsigned MAX < 1 is not taken: deleted"
        );
    }

    /// Flags from checked arithmetic (`adds`) are never trusted — the
    /// overflow guard and its raise path survive untouched.
    #[test]
    fn checked_arithmetic_guards_never_fold() {
        let mut stream = vec![
            mov_imm("%v1", "1"),
            mov_imm("%v2", "2"),
            ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b.vc", &[("target", "ok")]),
            ci("bl", &[("target", "_raise")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        run(&mut stream, 2);
        assert_eq!(ops(&stream), before);
    }

    /// An intervening unknown op (a call) invalidates the compare fact; an
    /// intervening *pure* op does not.
    #[test]
    fn flag_fact_survives_pure_ops_only() {
        let mut survives = vec![
            mov_imm("%v1", "3"),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "3")]),
            ci("add_imm", &[("dst", "%v9"), ("src", "%v1"), ("imm", "1")]),
            ci("b.eq", &[("target", "t")]),
            ci("label", &[("name", "t")]),
            ci("ret", &[]),
        ];
        run(&mut survives, 2);
        assert_eq!(survives[3].op, CodeOp::Branch, "pure op keeps the fact");

        let mut invalidated = vec![
            mov_imm("%v1", "3"),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "3")]),
            ci("bl", &[("target", "callee")]),
            ci("b.eq", &[("target", "t")]),
            ci("label", &[("name", "t")]),
            ci("ret", &[]),
        ];
        run(&mut invalidated, 2);
        assert_eq!(invalidated[3].op, CodeOp::BranchEq, "call clears the fact");
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let mut stream = vec![
            mov_imm("%v1", "3"),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "3")]),
            ci("b.eq", &[("target", "t")]),
            ci("label", &[("name", "t")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 1);
        assert_eq!(stream[2].op, CodeOp::BranchEq);
    }
}
