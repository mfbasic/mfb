//! The range-driven check-elision rows — seven Level-3 Opt2 catalog rows
//! (`planning/optimizations.md`) that share one engine, the integer range
//! lattice with dominating-predicate refinement in [`super::plans::ranges`]:
//!
//! - **Correlated value propagation** — refine a value from the branch
//!   conditions that dominate it, and use the refinement.
//! - **Overflow-check elimination** — an `adds`/`subs` whose input intervals
//!   cannot sum past the 64-bit range never sets `V`, so its `b.vc`/`b.vs`
//!   guard is decided.
//! - **Division / modulo-check elimination** — the `x != MIN`, `y != -1` and
//!   `y != 0` guards `emit_integer_division_overflow_check` and
//!   `emit_nonzero_or_invalid` emit.
//! - **Bounds-check elimination** — an index test whose failing edge raises
//!   `ErrIndexOutOfRange`.
//! - **Redundant union-tag / error-tag check elimination** — a discriminant
//!   test a dominating equality already settled.
//! - **Range-check widening / narrowing** — a check discharged by a fact
//!   *derived* through arithmetic from another value's fact (one dominating
//!   `i < n` proving `i`, `i + 1`, `i * 2` all in range).
//! - **Dead error-handler / fallible-branch elimination** — any other guard
//!   whose failing edge raises, proven never taken.
//!
//! They are one pass because they are one question asked seven ways: *is this
//! branch's outcome already determined by what is known here?* Building the
//! lattice seven times over would be seven times the cost for the same facts,
//! and would let the rows disagree with each other.
//!
//! **Attribution.** Each rewrite increments exactly one row's counter, chosen
//! by this fixed priority so `-v` totals stay a partition rather than
//! double-counting a shared mechanism:
//!
//! 1. an `adds`/`subs` overflow guard → overflow-check elimination;
//! 2. a target in the division-guard label family → division / modulo-check
//!    elimination;
//! 3. a failing edge that raises `ErrIndexOutOfRange` → bounds-check
//!    elimination;
//! 4. a failing edge that raises anything else → dead error-handler
//!    elimination;
//! 5. a deciding fact that came through arithmetic → range-check widening /
//!    narrowing;
//! 6. an equality/inequality test settled by a dominating equality →
//!    redundant union-tag / error-tag check elimination;
//! 7. anything else → correlated value propagation.
//!
//! **Trap discipline.** Deleting a guard is only sound because the lattice
//! refuses to model any operation that can raise: `adds`/`subs` contribute an
//! interval only when the true result is representable, and `b.vs`/`b.vc`
//! never *produce* a fact (they are trap flow, not a comparison). So the only
//! guards this row removes are ones it has an arithmetic proof for. A guard it
//! cannot prove is left exactly where it is, and the trap fires as written.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{
    build_cfg, classify_ref, Block, ClassModel, RegRef,
};
use crate::codegen::engine::regalloc::{self};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::mark::{conditional_terminator, flag_preserving, removable_op};
use super::plans::ranges::{
    self, block_compare, decide, relation_on_edge, Operandish, Range, Ranges,
};
use super::plans::ssa::{self, Ssa};

/// Which catalog row a rewrite is attributed to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Attribution {
    Overflow,
    Division,
    Bounds,
    DeadHandler,
    Widening,
    TagCheck,
    Correlated,
}

/// The label prefixes the two division guards mint
/// (`emit_integer_division_overflow_check`, `emit_nonzero_or_invalid`). Each
/// is minted at exactly one site, so a target carrying one identifies the
/// guard family unambiguously.
const DIVISION_GUARD_LABELS: [&str; 3] = ["div_not_min", "div_overflow_ok", "nonzero"];

/// Run the check-elision rows over one function's selected stream, in place.
/// Self-guarded on the shared catalog level (3).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);
    let facts = ranges::analyze(instructions, &models, &overlay, &blocks);
    let block_of = ranges::block_of(&blocks, instructions.len());

    // index -> Some(replacement) = rewrite; None = delete.
    let mut rewrites: Vec<(usize, Option<CodeInstruction>, Attribution)> = Vec::new();

    // (1) Correlated value propagation, value half: an instruction whose
    //     refined result is a single value becomes that value. Only the pure
    //     whitelist — never a flag-setter, whose flags a guard may read.
    for i in 0..instructions.len() {
        if !removable_op(instructions[i].op) || instructions[i].op == CodeOp::MovImm {
            continue;
        }
        let Some(RegRef::VReg(register)) = instructions[i]
            .operand("dst")
            .and_then(|operand| classify_ref(operand, &models.0))
        else {
            continue;
        };
        let Some(value) = overlay.value_defined_at(i, (false, register)) else {
            continue;
        };
        let block = block_of[i];
        let Some(pinned) = facts.at(block, value).singleton() else {
            continue;
        };
        // A `mov` of an already-known constant is the propagation rows' job;
        // this row only claims the ones the *refinement* proved.
        if !facts.is_derived(block, value) {
            continue;
        }
        let destination = instructions[i]
            .operand("dst")
            .expect("checked just above")
            .rendered()
            .into_owned();
        rewrites.push((
            i,
            // Spelled as the unsigned 64-bit pattern, the spelling every
            // other immediate in this seam uses (`emit_integer_pow` writes a
            // negative literal the same way); a bare `-1` is not what the
            // encoders read.
            Some(abi::move_immediate(
                &destination,
                "Integer",
                &(pinned as u64).to_string(),
            )),
            Attribution::Correlated,
        ));
    }

    // (2) The guard folds.
    for (index, block) in blocks.iter().enumerate() {
        let terminator = block.end - 1;
        let branch = instructions[terminator].op;
        if !conditional_terminator(branch) {
            continue;
        }
        let decided = match branch {
            CodeOp::BranchVc | CodeOp::BranchVs => {
                match overflow_guard_cannot_fire(
                    instructions,
                    &models,
                    &overlay,
                    &facts,
                    index,
                    block,
                ) {
                    // `b.vc` branches when no overflow happened, `b.vs` when
                    // one did.
                    true => Some((branch == CodeOp::BranchVc, Attribution::Overflow)),
                    false => None,
                }
            }
            _ => decide_comparison(instructions, &models, &overlay, &facts, index, block),
        };
        let Some((taken, attribution)) = decided else {
            continue;
        };

        // Refine the attribution from what the now-dead edge would have run.
        let attribution = match attribution {
            Attribution::Overflow => Attribution::Overflow,
            other => {
                let target = instructions[terminator].get("target").unwrap_or_default();
                if DIVISION_GUARD_LABELS
                    .iter()
                    .any(|prefix| target.starts_with(prefix))
                {
                    Attribution::Division
                } else {
                    match dead_edge_raises(instructions, &blocks, block, taken) {
                        Some(true) => Attribution::Bounds,
                        Some(false) => Attribution::DeadHandler,
                        None => other,
                    }
                }
            }
        };

        rewrites.push((
            terminator,
            taken.then(|| {
                let target = instructions[terminator]
                    .operand("target")
                    .cloned()
                    .expect("conditional branches carry a target");
                CodeInstruction::new("b").field("target", target)
            }),
            attribution,
        ));
    }

    if rewrites.is_empty() {
        return;
    }
    let mut counts = [0u64; 7];
    let mut keep = vec![true; instructions.len()];
    for (index, replacement, attribution) in rewrites {
        counts[attribution as usize] += 1;
        match replacement {
            Some(instruction) => instructions[index] = instruction,
            None => keep[index] = false,
        }
    }
    let mut index = 0;
    instructions.retain(|_| {
        let keep = keep[index];
        index += 1;
        keep
    });

    let stats = crate::optimizer::stats::CheckElisions {
        overflow: counts[Attribution::Overflow as usize],
        division: counts[Attribution::Division as usize],
        bounds: counts[Attribution::Bounds as usize],
        dead_handler: counts[Attribution::DeadHandler as usize],
        widening: counts[Attribution::Widening as usize],
        tag: counts[Attribution::TagCheck as usize],
        correlated: counts[Attribution::Correlated as usize],
    };
    crate::optimizer::stats::count_check_elisions(stats);
}

/// Whether the `adds`/`subs` this block's overflow guard tests provably
/// cannot set `V`.
///
/// Only the flag-setter immediately governing the branch counts: nothing but
/// provably flag-free instructions may sit between them, the same rule the
/// branch-folding row uses.
fn overflow_guard_cannot_fire(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    facts: &Ranges,
    index: usize,
    block: &Block,
) -> bool {
    let mut i = block.end - 1;
    while i > block.start {
        i -= 1;
        let op = instructions[i].op;
        match op {
            CodeOp::Adds | CodeOp::Subs => {
                let lhs = operand_range(instructions, models, overlay, facts, index, i, "lhs");
                let rhs = operand_range(instructions, models, overlay, facts, index, i, "rhs");
                return match op {
                    CodeOp::Adds => ranges::add_cannot_overflow(lhs, rhs),
                    _ => ranges::sub_cannot_overflow(lhs, rhs),
                };
            }
            _ if flag_preserving(&instructions[i]) => continue,
            // Any other flag-toucher (or an unmodeled instruction that might
            // be one): no proof.
            _ => return false,
        }
    }
    false
}

/// Whether the block's compare-and-branch has an outcome the facts settle,
/// and which row the removal belongs to on the "how it was proven" axis.
fn decide_comparison(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    facts: &Ranges,
    index: usize,
    block: &Block,
) -> Option<(bool, Attribution)> {
    let compare = block_compare(instructions, models, overlay, block)?;
    let branch = instructions[block.end - 1].op;
    let side = |operandish: Operandish| -> Option<(Range, bool, bool)> {
        match operandish {
            Operandish::Value(value) => Some((
                facts.at(index, value),
                facts.is_derived(index, value),
                facts.at(index, value).singleton().is_some(),
            )),
            Operandish::Literal(value) => Some((Range::exact(value), false, true)),
            Operandish::Opaque => None,
        }
    };
    let (lhs, lhs_derived, lhs_pinned) = side(compare.lhs)?;
    let (rhs, rhs_derived, rhs_pinned) = side(compare.rhs)?;
    let relation = relation_on_edge(branch, true, lhs, rhs)?;
    let taken = decide(relation, lhs, rhs)?;
    let attribution = if lhs_derived || rhs_derived {
        Attribution::Widening
    } else if matches!(branch, CodeOp::BranchEq | CodeOp::BranchNe) && (lhs_pinned || rhs_pinned) {
        Attribution::TagCheck
    } else {
        Attribution::Correlated
    };
    Some((taken, attribution))
}

/// The interval one operand of the instruction at `i` holds, read with the
/// facts of `block` (the block `i` sits in).
fn operand_range(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    facts: &Ranges,
    block: usize,
    i: usize,
    field: &str,
) -> Range {
    let Some(operand) = instructions[i].operand(field) else {
        return Range::FULL;
    };
    match classify_ref(operand, &models.0) {
        Some(RegRef::VReg(id)) => match overlay.value_of_use(i, (false, id)) {
            Some(value) => facts.at(block, value),
            None => Range::FULL,
        },
        Some(RegRef::Phys(_)) => Range::FULL,
        None => match ranges::literal(&operand.rendered()) {
            Some(value) => Range::exact(value),
            None => Range::FULL,
        },
    }
}

/// The symbol every trap site calls to assemble its `Error` result. A block
/// reaching it is a raise path.
const ERROR_ASSEMBLY_SYMBOL: &str = "_mfb_make_error_result";

/// How far into the dead block to look for the raise. `raise_error_bare`
/// emits the code immediate and the five argument loads directly, so the call
/// is a handful of instructions in.
const RAISE_WINDOW: usize = 24;

/// Whether the edge this fold kills led to a raise, and if so whether the
/// error raised is `ErrIndexOutOfRange` (`Some(true)` — a bounds check) or
/// some other error (`Some(false)`). `None` means the dead edge is ordinary
/// control flow, not a check at all.
fn dead_edge_raises(
    instructions: &[CodeInstruction],
    blocks: &[Block],
    block: &Block,
    taken: bool,
) -> Option<bool> {
    // Taken means the fall-through died, and vice versa.
    let dead = if taken {
        *block.succ.get(1)?
    } else {
        *block.succ.first()?
    };
    let dead = blocks.get(dead)?;
    let end = dead.end.min(dead.start + RAISE_WINDOW);
    let index_code =
        crate::codegen::registry::runtime_error("ErrIndexOutOfRange").map(|(code, _)| code);
    let mut saw_index_code = false;
    for instruction in &instructions[dead.start..end] {
        if instruction.op == CodeOp::MovImm
            && instruction.get("type").as_deref() == Some("Integer")
            && index_code.is_some()
            && instruction.get("value").as_deref() == index_code
        {
            saw_index_code = true;
        }
        let calls_error_assembly = matches!(instruction.op, CodeOp::BranchLink)
            && instruction
                .get("target")
                .is_some_and(|target| target.contains(ERROR_ASSEMBLY_SYMBOL));
        if calls_error_assembly {
            return Some(saw_index_code);
        }
    }
    None
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
        let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
        with_opt_level(OptLevel(level), || eliminate(stream, &model));
    }

    /// A checked add of two small constants cannot overflow, so its `b.vc`
    /// guard is always taken and the raise path is orphaned.
    #[test]
    fn a_provably_safe_checked_add_loses_its_guard() {
        let mut stream = vec![
            mov_imm("%v1", "2"),
            mov_imm("%v2", "3"),
            ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b.vc", &[("target", "overflow_ok_1")]),
            ci("bl", &[("target", "_mfb_make_error_result")]),
            ci("label", &[("name", "overflow_ok_1")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            stream[3].op,
            CodeOp::Branch,
            "the guard is unconditional now"
        );
    }

    /// A checked add whose operands the lattice cannot bound keeps its guard.
    #[test]
    fn an_unprovable_checked_add_keeps_its_guard() {
        let stream = || {
            vec![
                ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.vc", &[("target", "overflow_ok_1")]),
                ci("bl", &[("target", "_mfb_make_error_result")]),
                ci("label", &[("name", "overflow_ok_1")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()), "no proof, no removal");
    }

    /// A dominating `i < 10` discharges a later `i < 20`.
    #[test]
    fn a_dominating_bound_discharges_a_weaker_one() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "10")]),
            ci("b.ge", &[("target", "out")]),
            // %v1 <= 9 here, so `%v1 < 20` is certain.
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "20")]),
            ci("b.lt", &[("target", "ok")]),
            ci("bl", &[("target", "_mfb_make_error_result")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
            ci("label", &[("name", "out")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[3].op, CodeOp::Branch, "the second test is settled");
    }

    /// A genuinely undecided comparison is untouched.
    #[test]
    fn an_undecided_comparison_survives() {
        let stream = || {
            vec![
                ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "10")]),
                ci("b.ge", &[("target", "out")]),
                ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "5")]),
                ci("b.lt", &[("target", "ok")]),
                ci("bl", &[("target", "_mfb_make_error_result")]),
                ci("label", &[("name", "ok")]),
                ci("ret", &[]),
                ci("label", &[("name", "out")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// The whole row is off below `-O3`.
    #[test]
    fn level_two_disables_the_rows() {
        let stream = || {
            vec![
                mov_imm("%v1", "2"),
                mov_imm("%v2", "3"),
                ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.vc", &[("target", "overflow_ok_1")]),
                ci("bl", &[("target", "_mfb_make_error_result")]),
                ci("label", &[("name", "overflow_ok_1")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 2);
        assert_eq!(ops(&off), ops(&stream()));
    }
}
