//! Redundant load elimination — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): remove a load of a value already available
//! in a register because an *earlier load* of the same stack slot put it
//! there, on every path.
//!
//! Store-to-load forwarding's sibling: both consume the same
//! [`super::plans::memory`] availability dataflow and differ only in the
//! origin of the available value. Here the origin is an earlier `ldr`, so the
//! reload is pure re-fetching of memory nothing has written in between — the
//! dataflow proves exactly that, since any instruction it does not model
//! (a call, another store, an FP or unknown op) clears the whole state, and
//! the meet keeps a slot only when every predecessor path agrees on the same
//! SSA value. The holder register must additionally be single-def, the same
//! rule GVN uses, so the register still holds what the analysis recorded.
//!
//! The rewrite is `ldr dst, [sp,#off]` → `mov dst, <holder>`; copy
//! propagation then bypasses the copy and DCE sweeps the strands. The pass
//! body lives with its sibling ([`super::stldfwd`]): one traversal serves
//! both rows, each reporting into its own counter. Broadening
//! past `sp` slots to arbitrary bases needs Plan2 alias analysis / memory-SSA
//! (the catalog's "Prerequisites are not dial rows" note), so this row covers
//! the frame traffic the storage planner actually emits.

#[cfg(test)]
mod tests {
    // The row's rewrites are applied by the shared traversal in
    // `super::stldfwd` (see the module docs); these are this row's own tests
    // of the load-sourced half.
    use super::super::stldfwd::forward;
    use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::types::CodeInstruction;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn run(stream: &mut [CodeInstruction], level: u8) {
        with_opt_level(OptLevel(level), || forward(stream, &Aarch64RegisterModel));
    }

    /// A second load of the same slot, in a later block, reads the first
    /// load's register instead of memory.
    #[test]
    fn reloads_become_copies_across_blocks() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("b", &[("target", "next")]),
            ci("label", &[("name", "next")]),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("add", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci(
                "str_u64",
                &[("src", "%v3"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[3].op, CodeOp::Mov, "the reload became a copy");
        assert_eq!(stream[3].get("src").as_deref(), Some("%v1"));
    }

    /// A store to the same slot in between means the reload no longer reads
    /// the first load's value: the slot now holds the *stored* register, so
    /// the rewrite (if any) is the store-to-load row's and reads `%v9`, never
    /// the stale `%v1`.
    #[test]
    fn an_intervening_store_retargets_the_reload() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci(
                "str_u64",
                &[("src", "%v9"), ("base", "sp"), ("offset", "8")],
            ),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_ne!(
            stream[2].get("src").as_deref(),
            Some("%v1"),
            "the first load's value is stale after the store"
        );
    }

    /// A call between the loads ends availability — it may write the frame.
    #[test]
    fn calls_stop_load_reuse() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("bl", &[("target", "_mfb_fn_callee")]),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[2].op, CodeOp::LdrU64);
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = vec![
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[1].op, CodeOp::LdrU64);
    }
}
