//! Induction variable simplification — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): canonicalize a loop's counters by merging
//! *duplicate* induction variables, on the SSA overlay
//! ([`super::plans::ssa`]).
//!
//! Lowering routinely mints more than one counter for the same loop — a user
//! `FOR` variable alongside the iteration temp, an index and a cursor stepping
//! in lockstep. Two basic induction variables that provably hold the same
//! value at every point are redundant: the row rewrites uses of the second to
//! read the first, and the now-unused counter (its phi and its increment)
//! dies in the DCE row.
//!
//! What "provably the same" requires here — all four, or the pair is skipped:
//!
//! 1. both are **basic induction variables** of the same loop header: a phi
//!    in that header with exactly two arguments, one from outside the loop
//!    (the initial value) and one produced by an `add_imm` reading that same
//!    phi (the step);
//! 2. the initial arguments are the **same value** (compared through GVN's
//!    copy/constant canonicalization, so two `mov`s of one source count as
//!    equal starts) and the steps are the same constant — so they start equal
//!    and advance equally;
//! 3. both increments live in the **same block**, so they execute the same
//!    number of times (a conditionally-updated counter would drift);
//! 4. the survivor's register has exactly **two definitions** in the whole
//!    stream — the initialization and that increment — so wherever its
//!    increment has run the register provably holds the counter's current
//!    value (the same holder-currency argument GVN makes with its single-def
//!    rule).
//!
//! Only uses *after both increments* are rewritten (in that block, or in one
//! it strictly dominates), and only uses reading the duplicate's
//! post-increment value: between the two increments the counters legitimately
//! differ by one step, and rewriting there would be a miscompile.
//!
//! Trap discipline is structural: `add_imm` is in the pure, flag-free
//! whitelist, so a checked `adds` counter (which can raise `ErrOverflow`) is
//! never a candidate, and nothing is added, removed, or reordered here — only
//! a use's register spelling changes, to a register holding the identical
//! value. Strength-reducing *derived* induction variables (`iv * C` into an
//! add chain) is the row's larger future half and needs a cost model plus
//! new instructions; it is deliberately not attempted.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::{
    build_cfg, classify_ref, effect, is_use_field, RegRef,
};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::branches::bits;
use super::gvn::canonical;
use super::lvn::KeyPart;
use super::plans::ssa::{self, ValueDef, ValueId, Var};

/// One basic induction variable of a loop header.
struct Induction {
    /// The variable (vreg) the counter lives in.
    var: Var,
    /// The value entering from outside the loop, canonicalized through copy
    /// chains and constants — two counters started by separate `mov`s of one
    /// source do begin equal.
    init: KeyPart,
    /// The constant added each iteration.
    step: u64,
    /// The increment instruction (its block must match a peer's).
    increment: usize,
    /// The increment's block.
    block: usize,
    /// The counter register's operand spelling, for rewriting uses.
    operand: Operand,
}

/// Run the induction-variable row over one function's selected stream, in
/// place. Self-guarded on the row's catalog level (3).
pub(crate) fn simplify(instructions: &mut [CodeInstruction], model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        crate::optimizer::stats::count_induction_vars_merged(0);
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);

    let mut block_of = vec![0usize; instructions.len()];
    for (index, block) in blocks.iter().enumerate() {
        for slot in &mut block_of[block.start..block.end] {
            *slot = index;
        }
    }
    let mut def_count: HashMap<Var, u32> = HashMap::new();
    for instruction in instructions.iter() {
        for class_model in [&models.0, &models.1] {
            for def in effect(instruction, class_model).defs {
                if let RegRef::VReg(id) = def {
                    *def_count.entry((class_model.is_fp, id)).or_insert(0) += 1;
                }
            }
        }
    }

    // Collect the basic induction variables, grouped by header block.
    let mut by_header: HashMap<usize, Vec<Induction>> = HashMap::new();
    for (vid, def) in overlay.values.iter().enumerate() {
        let ValueDef::Phi { block, args } = def else {
            continue;
        };
        if args.len() != 2 {
            continue;
        }
        // Exactly one argument must be the header's own `add_imm` step; the
        // other is the initial value from outside the loop.
        for (step_index, other_index) in [(0usize, 1usize), (1, 0)] {
            let (_, step_value) = args[step_index];
            let (_, init) = args[other_index];
            let ValueDef::Inst(increment) = overlay.values[step_value] else {
                continue;
            };
            let instruction = &instructions[increment];
            if instruction.op != CodeOp::AddImm {
                continue;
            }
            // The increment must read this very phi (a self-referential step)
            // and its destination must be the counter's own register.
            let Some(dst_operand) = instruction.operand("dst") else {
                continue;
            };
            let Some(RegRef::VReg(dst_id)) = classify_ref(dst_operand, &models.0) else {
                continue;
            };
            let var = (false, dst_id);
            let reads_self = instruction
                .operand("src")
                .and_then(|src| match classify_ref(src, &models.0) {
                    Some(RegRef::VReg(id)) => overlay.value_of_use(increment, (false, id)),
                    _ => None,
                })
                .is_some_and(|used| used == vid);
            if !reads_self {
                continue;
            }
            let Some(step) = instruction.get("imm").and_then(|text| bits(&text)) else {
                continue;
            };
            by_header.entry(*block).or_default().push(Induction {
                var,
                init: canonical(init, instructions, &models, &overlay),
                step,
                increment,
                block: block_of[increment],
                operand: dst_operand.clone(),
            });
            break;
        }
    }

    // Merge duplicates. A redirect is valid only *after both* counters have
    // stepped: between the two increments the duplicate and the survivor
    // differ by one, so only uses that follow the later increment (in its
    // block, or in a block it strictly dominates) may be rewritten — and only
    // uses that read the duplicate's post-increment value, which is exactly
    // what the survivor's register holds there.
    struct Redirect {
        survivor: Operand,
        block: usize,
        after: usize,
    }
    let mut redirect: HashMap<ValueId, Redirect> = HashMap::new();
    for candidates in by_header.values_mut() {
        // Deterministic survivor: the lowest-numbered counter register wins,
        // never "whichever phi got the lower value id" (an artifact of
        // analysis ordering, and so a source of unstable codegen).
        candidates.sort_by_key(|candidate| candidate.var);
        for (index, candidate) in candidates.iter().enumerate() {
            // The survivor's register must hold the counter and nothing else:
            // exactly the initialization and that increment define it.
            if def_count.get(&candidate.var) != Some(&2) {
                continue;
            }
            for duplicate in candidates.iter().skip(index + 1) {
                if duplicate.var == candidate.var
                    || duplicate.init != candidate.init
                    || duplicate.step != candidate.step
                    || duplicate.block != candidate.block
                    || duplicate.increment == candidate.increment
                {
                    continue;
                }
                let Some(step_value) = overlay.value_defined_at(duplicate.increment, duplicate.var)
                else {
                    continue;
                };
                redirect.entry(step_value).or_insert(Redirect {
                    survivor: candidate.operand.clone(),
                    block: candidate.block,
                    after: candidate.increment.max(duplicate.increment),
                });
            }
        }
    }
    if redirect.is_empty() {
        crate::optimizer::stats::count_induction_vars_merged(0);
        return;
    }

    // Rewrite the eligible uses — never a definition field, and never inside
    // the increments themselves.
    let mut fired = 0;
    for i in 0..instructions.len() {
        let mut replacements: Vec<(usize, Operand)> = Vec::new();
        for (slot, (name, operand)) in instructions[i].fields.iter().enumerate() {
            if !is_use_field(name) {
                continue;
            }
            let Some(RegRef::VReg(id)) = classify_ref(operand, &models.0) else {
                continue;
            };
            let Some(value) = overlay.value_of_use(i, (false, id)) else {
                continue;
            };
            let Some(entry) = redirect.get(&value) else {
                continue;
            };
            let in_range = (block_of[i] == entry.block && i > entry.after)
                || (block_of[i] != entry.block && overlay.dominates(entry.block, block_of[i]));
            if in_range && entry.survivor.rendered() != operand.rendered() {
                replacements.push((slot, entry.survivor.clone()));
            }
        }
        for (slot, survivor) in replacements {
            instructions[i].fields[slot].1 = survivor;
            fired += 1;
        }
    }
    crate::optimizer::stats::count_induction_vars_merged(fired);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn run(stream: &mut [CodeInstruction], level: u8) {
        with_opt_level(OptLevel(level), || simplify(stream, &Aarch64RegisterModel));
    }

    /// Two counters starting from the same value and stepping by the same
    /// constant in the same block are duplicates: uses of the second read the
    /// first, leaving the second's increment for DCE.
    fn twin_counter_loop(second_step: &str, second_init: &str) -> Vec<CodeInstruction> {
        vec![
            /* 0 */ ci("mov", &[("dst", "%v1"), ("src", "%v9")]),
            /* 1 */ ci("mov", &[("dst", "%v2"), ("src", second_init)]),
            /* 2 */ ci("label", &[("name", "head")]),
            /* 3 */ ci("add_imm", &[("dst", "%v1"), ("src", "%v1"), ("imm", "1")]),
            /* 4 */
            ci(
                "add_imm",
                &[("dst", "%v2"), ("src", "%v2"), ("imm", second_step)],
            ),
            /* 5 */
            ci(
                "str_u64",
                &[("src", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            /* 6 */ ci("cmp", &[("lhs", "%v1"), ("rhs", "%v8")]),
            /* 7 */ ci("b.ne", &[("target", "head")]),
            /* 8 */ ci("ret", &[]),
        ]
    }

    #[test]
    fn duplicate_counters_merge() {
        let mut stream = twin_counter_loop("1", "%v9");
        run(&mut stream, 3);
        assert_eq!(
            stream[5].get("src").as_deref(),
            Some("%v1"),
            "the store reads the surviving counter"
        );
        // The redundant counter's own increment still reads itself (it is now
        // dead code, which the DCE row removes).
        assert_eq!(stream[4].get("dst").as_deref(), Some("%v2"));
    }

    /// Different stride or different initial value means the counters differ:
    /// nothing is merged.
    #[test]
    fn different_stride_or_init_blocks_the_merge() {
        let mut different_step = twin_counter_loop("2", "%v9");
        run(&mut different_step, 3);
        assert_eq!(different_step[5].get("src").as_deref(), Some("%v2"));

        let mut different_init = twin_counter_loop("1", "%v7");
        run(&mut different_init, 3);
        assert_eq!(different_init[5].get("src").as_deref(), Some("%v2"));
    }

    /// A counter updated in a different block runs a different number of
    /// times: not a duplicate.
    #[test]
    fn conditionally_updated_counters_are_not_duplicates() {
        let mut stream = vec![
            ci("mov", &[("dst", "%v1"), ("src", "%v9")]),
            ci("mov", &[("dst", "%v2"), ("src", "%v9")]),
            ci("label", &[("name", "head")]),
            ci("add_imm", &[("dst", "%v1"), ("src", "%v1"), ("imm", "1")]),
            ci("b.eq", &[("target", "skip")]),
            ci("add_imm", &[("dst", "%v2"), ("src", "%v2"), ("imm", "1")]),
            ci("label", &[("name", "skip")]),
            ci(
                "str_u64",
                &[("src", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("cmp", &[("lhs", "%v1"), ("rhs", "%v8")]),
            ci("b.ne", &[("target", "head")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[7].get("src").as_deref(), Some("%v2"));
    }

    /// Checked arithmetic (`adds`, which can raise `ErrOverflow`) is never an
    /// induction-variable candidate.
    #[test]
    fn checked_counters_are_not_candidates() {
        let mut stream = vec![
            ci("mov", &[("dst", "%v1"), ("src", "%v9")]),
            ci("mov", &[("dst", "%v2"), ("src", "%v9")]),
            ci("label", &[("name", "head")]),
            ci("adds", &[("dst", "%v1"), ("lhs", "%v1"), ("rhs", "%v3")]),
            ci("adds", &[("dst", "%v2"), ("lhs", "%v2"), ("rhs", "%v3")]),
            ci(
                "str_u64",
                &[("src", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("b.ne", &[("target", "head")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[5].get("src").as_deref(), Some("%v2"));
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = twin_counter_loop("1", "%v9");
        run(&mut stream, 2);
        assert_eq!(stream[5].get("src").as_deref(), Some("%v2"));
    }
}
