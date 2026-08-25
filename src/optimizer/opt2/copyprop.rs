//! Copy propagation — a Level-2 Opt2 catalog row
//! (`planning/optimizations.md`): rewrite a use of a copied register to read
//! the copy's source directly, on the SSA overlay ([`super::plans::ssa`]).
//!
//! All the analysis lives in the overlay: during its renaming walk it records,
//! for every use whose SSA value was produced by a register-to-register copy
//! (`mov` / `fmov_d_from_d`), the copy chain's *ultimate* source operand —
//! and only when that source provably still holds the same SSA value at the
//! use point (a redefinition on any intervening path surfaces as a phi and
//! invalidates the forward). This pass is then a pure field substitution over
//! [`Ssa::forwarded_source`], touching only use fields (never `dst`), which
//! is trivially behavior-preserving: the replacement register holds the
//! identical bits at that point, no instruction is added, removed, or
//! reordered, and the bypassed copies become dead for the DCE row that runs
//! after. Register pressure is the only cost (a forwarded source lives
//! longer), and the allocator re-runs full liveness on the rewritten stream.

use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::{build_cfg, classify_ref, is_use_field, RegRef};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::ssa;

/// Run the copy-propagation row over one function's selected stream, in
/// place. Self-guarded on the row's catalog level (2).
pub(crate) fn eliminate(instructions: &mut [CodeInstruction], model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    let overlay = ssa::build(instructions, &blocks, &models);

    let mut fired = 0;
    for (i, instruction) in instructions.iter_mut().enumerate() {
        for (name, value) in instruction.fields.iter_mut() {
            if !is_use_field(name) {
                continue;
            }
            for class_model in [&models.0, &models.1] {
                if let Some(RegRef::VReg(id)) = classify_ref(value, class_model) {
                    if let Some(source) = overlay.forwarded_source(i, (class_model.is_fp, id)) {
                        *value = source.clone();
                        fired += 1;
                    }
                    break; // a register classifies under exactly one class
                }
            }
        }
    }
    crate::optimizer::stats::count_copy_propagations(fired);
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
        with_opt_level(OptLevel(level), || eliminate(stream, &Aarch64RegisterModel));
    }

    /// A copied register's uses read the source directly — across a label,
    /// which no block-local pass could cross — and a copy-of-a-copy chain
    /// collapses to the original. The bypassed `mov`s become DCE's food.
    #[test]
    fn uses_read_through_the_copy_chain() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            ci("label", &[("name", "next")]),
            ci("mov", &[("dst", "%v3"), ("src", "%v2")]),
            ci("add", &[("dst", "%v4"), ("lhs", "%v3"), ("rhs", "%v2")]),
            ci(
                "str_u64",
                &[("src", "%v4"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[4].get("lhs").as_deref(), Some("%v1"));
        assert_eq!(stream[4].get("rhs").as_deref(), Some("%v1"));
        assert_eq!(
            stream[3].get("src").as_deref(),
            Some("%v1"),
            "the inner copy's own read also collapses"
        );
    }

    /// A source redefined on one path must not be forwarded past the join,
    /// and a `dst` field is never rewritten even when its register has a
    /// forwardable use elsewhere in the same instruction.
    #[test]
    fn redefined_sources_and_dst_fields_are_untouched() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            ci("b.eq", &[("target", "join")]),
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "0")],
            ),
            ci("label", &[("name", "join")]),
            // %v2's use must stay %v2 (its source %v1 may be 0 here), and the
            // dst %v2 must never be substituted.
            ci("add", &[("dst", "%v2"), ("lhs", "%v2"), ("rhs", "%v2")]),
            ci(
                "str_u64",
                &[("src", "%v2"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        let before: Vec<String> = stream.iter().map(|i| format!("{:?}", i.op)).collect();
        run(&mut stream, 2);
        assert_eq!(stream[5].get("lhs").as_deref(), Some("%v2"));
        assert_eq!(stream[5].get("rhs").as_deref(), Some("%v2"));
        assert_eq!(stream[5].get("dst").as_deref(), Some("%v2"));
        let after: Vec<String> = stream.iter().map(|i| format!("{:?}", i.op)).collect();
        assert_eq!(before, after, "no instruction added or removed");
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            ci(
                "str_u64",
                &[("src", "%v2"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 1);
        assert_eq!(stream[2].get("src").as_deref(), Some("%v2"));
    }
}
