//! Constant propagation — a Level-2 Opt2 catalog row
//! (`planning/optimizations.md`): replace registers known to hold constants
//! across the whole function, on the SSA overlay ([`super::plans::ssa`]).
//!
//! The Level-1 constant folder ([`super::constant_folding`]) already folds
//! within a block; this row is its global form. A constant lattice is
//! computed per SSA *value* — an instruction value evaluates through the very
//! same [`constant_folding::fold_one`] rules (wrapping 64-bit machine ALU,
//! `mov_imm` Integer feeds, `mov` passthrough; nothing else, so the two rows
//! cannot drift), and a phi is constant when every argument is the same
//! constant — which is exactly what lets a constant survive a join the
//! block-local folder must forget at (`IF`/`ELSE` arms assigning the same
//! value, lowering-introduced diamonds). The fixpoint is pessimistic: a value
//! is constant only when proven from already-proven inputs, so loop-carried
//! phis simply stay unknown.
//!
//! **Trap preservation for free**, same argument as the folder: MFB's checked
//! arithmetic lowers to flag-setting ops `fold_one` refuses to model
//! (`Step::Barrier`), so only the raw wrapping machine ops lowering itself
//! emits are ever rewritten. A rewrite replaces a pure modeled instruction
//! with `mov_imm dst, <constant>` — value-identical bits in the destination,
//! no flags, no memory — and the stranded feeders are the DCE row's job.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::{build_cfg, classify_ref, RegRef};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;
use crate::target::shared::regmodel::RegisterModel;

use super::constant_folding::{fold_one, Step};
use super::plans::ssa::{self, Ssa, ValueDef};

/// Run the constant-propagation row over one function's selected stream, in
/// place. Self-guarded on the row's catalog level (2).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    let overlay = ssa::build(instructions, &blocks, &models);

    // The constant each SSA value provably holds. Pessimistic fixpoint: only
    // integer-class GPR constants enter the domain (the folder's rule).
    let mut const_of: Vec<Option<u64>> = vec![None; overlay.values.len()];
    // A register field's constant at instruction `i`: its int-class use's SSA
    // value's constant. FP and physical registers resolve to nothing.
    let resolve = |i: usize,
                   instruction: &CodeInstruction,
                   const_of: &[Option<u64>],
                   overlay: &Ssa,
                   field: &str|
     -> Option<u64> {
        let operand = instruction.operand(field)?;
        match classify_ref(operand, &models.0)? {
            RegRef::VReg(id) => const_of[overlay.value_of_use(i, (false, id))?],
            RegRef::Phys(_) => None,
        }
    };
    // Reverse dependency edges (value -> consumers), so resolving a value
    // re-attempts exactly its dependents — each value is evaluated at most
    // 1 + indegree times instead of once per fixpoint round (round-scanning
    // is quadratic on the huge generated functions' deep phi chains).
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); overlay.values.len()];
    for (vid, def) in overlay.values.iter().enumerate() {
        match def {
            ValueDef::Inst(i) => {
                for (name, operand) in &instructions[*i].fields {
                    if !crate::codegen::engine::regalloc::analysis::is_use_field(name) {
                        continue;
                    }
                    if let Some(RegRef::VReg(id)) = classify_ref(operand, &models.0) {
                        if let Some(w) = overlay.value_of_use(*i, (false, id)) {
                            dependents[w].push(vid);
                        }
                    }
                }
            }
            ValueDef::Phi { args, .. } => {
                for &(_, arg) in args {
                    dependents[arg].push(vid);
                }
            }
            ValueDef::Entry => {}
        }
    }
    let mut worklist: Vec<usize> = (0..overlay.values.len()).collect();
    while let Some(vid) = worklist.pop() {
        if const_of[vid].is_some() {
            continue;
        }
        let constant = match &overlay.values[vid] {
            ValueDef::Inst(i) => {
                let instruction = &instructions[*i];
                match fold_one(instruction, &|field| {
                    resolve(*i, instruction, &const_of, &overlay, field)
                }) {
                    Step::Record(_, value) | Step::Replace(_, value) => Some(value),
                    Step::KillDst | Step::Barrier => None,
                }
            }
            ValueDef::Phi { args, .. } => {
                let mut first: Option<u64> = None;
                let all_agree = !args.is_empty()
                    && args.iter().all(|&(_, arg)| match const_of[arg] {
                        Some(c) => {
                            if first.is_none() {
                                first = Some(c);
                            }
                            first == Some(c)
                        }
                        None => false,
                    });
                if all_agree {
                    first
                } else {
                    None
                }
            }
            ValueDef::Entry => None,
        };
        if constant.is_some() {
            const_of[vid] = constant;
            worklist.extend(dependents[vid].iter().copied());
        }
    }

    // Rewrite every modeled instruction whose result is now a proven
    // constant. `Replace` is the folder's own rewrite verdict; a `mov` whose
    // source is constant (`Record`) becomes a `mov_imm` too — that is the
    // propagation itself. `mov_imm` already is one.
    let mut fired = 0;
    for i in 0..instructions.len() {
        if instructions[i].op == CodeOp::MovImm {
            continue;
        }
        let step = {
            let instruction = &instructions[i];
            fold_one(instruction, &|field| {
                resolve(i, instruction, &const_of, &overlay, field)
            })
        };
        match step {
            Step::Replace(dst, value) => {
                instructions[i] = abi::move_immediate(&dst, "Integer", &value.to_string());
                fired += 1;
            }
            Step::Record(dst, value) if instructions[i].op == CodeOp::Mov => {
                instructions[i] = abi::move_immediate(&dst, "Integer", &value.to_string());
                fired += 1;
            }
            _ => {}
        }
    }
    crate::optimizer::stats::count_constant_propagations(fired);
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

    fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
        ci(
            "mov_imm",
            &[("dst", dst), ("type", "Integer"), ("value", value)],
        )
    }

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
        with_opt_level(OptLevel(level), || eliminate(stream, &Aarch64RegisterModel));
    }

    /// The case the block-local folder must refuse (knowledge cannot cross a
    /// label without a CFG): with SSA the label has one predecessor, the
    /// constant survives, and the add folds.
    #[test]
    fn constants_survive_a_single_predecessor_label() {
        let mut stream = vec![
            mov_imm("%v1", "2"),
            ci("label", &[("name", "next")]),
            ci("add", &[("dst", "%v2"), ("lhs", "%v1"), ("rhs", "%v1")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::MovImm);
        assert_eq!(stream[2].get("value").as_deref(), Some("4"));
    }

    /// A diamond whose two arms bind the same constant: the phi agrees, the
    /// use after the join folds.
    #[test]
    fn agreeing_phi_arguments_propagate() {
        let mut stream = vec![
            ci("b.eq", &[("target", "else")]),
            mov_imm("%v1", "5"),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "else")]),
            mov_imm("%v1", "5"),
            ci("label", &[("name", "join")]),
            ci("add_imm", &[("dst", "%v2"), ("src", "%v1"), ("imm", "1")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[6].op, CodeOp::MovImm, "5+1 folds through the join");
        assert_eq!(stream[6].get("value").as_deref(), Some("6"));
    }

    /// Disagreeing arms: the phi is not constant, nothing rewrites.
    #[test]
    fn disagreeing_phi_arguments_do_not_propagate() {
        let mut stream = vec![
            ci("b.eq", &[("target", "else")]),
            mov_imm("%v1", "5"),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "else")]),
            mov_imm("%v1", "6"),
            ci("label", &[("name", "join")]),
            ci("add_imm", &[("dst", "%v2"), ("src", "%v1"), ("imm", "1")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[6].op, CodeOp::AddImm, "join value is not constant");
    }

    /// A `mov` of a known constant becomes the constant — the propagation
    /// the row is named for.
    #[test]
    fn mov_of_a_constant_becomes_mov_imm() {
        let mut stream = vec![
            mov_imm("%v1", "9"),
            ci("label", &[("name", "l")]),
            ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::MovImm);
        assert_eq!(stream[2].get("value").as_deref(), Some("9"));
    }

    /// Checked arithmetic (flag-setting `adds`) is outside the fold model:
    /// its inputs may be constant but it is never rewritten.
    #[test]
    fn flag_setting_arithmetic_is_never_rewritten() {
        let mut stream = vec![
            mov_imm("%v1", "2"),
            mov_imm("%v2", "3"),
            ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b.vc", &[("target", "ok")]),
            ci("bl", &[("target", "_raise")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[2].op, CodeOp::Adds);
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let mut stream = vec![
            mov_imm("%v1", "2"),
            ci("label", &[("name", "next")]),
            ci("add", &[("dst", "%v2"), ("lhs", "%v1"), ("rhs", "%v1")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 1);
        assert_eq!(stream[2].op, CodeOp::Add);
    }

    #[test]
    fn physical_register_values_are_not_treated_as_constants() {
        let mut stream = vec![ci("mov", &[("dst", "%v1"), ("src", "x0")]), ci("ret", &[])];
        run(&mut stream, 2);
        assert_eq!(stream[0].op, CodeOp::Mov);
    }
}
