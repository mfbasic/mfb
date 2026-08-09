//! plan-71-B Phase 1: the Category-2 self-move probe.
//!
//! On AArch64 and RISC-V the argument and result ABI role tokens realize to the
//! *same* physical register (`%arg0` and `%ret0` → `x0`; see
//! [`crate::target::shared::abi::realize_abi_token`]), so a same-index staging
//! move `mov %argK,%retK` collapses to a `mov xN,xN` no-op after realization — an
//! instruction that does not exist on x86, where the two roles split across
//! `rdi`/`rax`. plan-71-E will lift that staging onto the shared lowering path;
//! before then this probe measures how many such self-moves the *current* codegen
//! already emits per target (plan-71-B Phase 1), which is the number that decides
//! whether the AArch64/RISC-V elision pass (Phase 3) is needed at all.
//!
//! The scan is pure and read-only: it returns the report lines and never mutates
//! the stream, so the selector wrappers env-gate the `eprintln!`
//! (`MFB_BUG387_SELFMOVE`) and every emitted byte is unchanged when it is unset —
//! the same discipline as plan-71-A's `remap_x86_abi` audit (inner returns lines;
//! the wrapper prints).

use crate::arch::ops::CodeOp;

use super::CodeInstruction;

/// Enumerate the `mov <reg>,<reg>` self-moves in a finalized instruction stream
/// (after ABI-token realization), one `BUG387-SELFMOVE` line per match. `target`
/// labels each line for the corpus sweep. Pure — the caller decides whether to
/// print, so a byte-identity build that never calls the printer is unaffected.
///
/// A `mov` is a self-move iff both operands render to the same spelling. Matching
/// on `CodeOp::Mov` (not the raw field strings) means a store whose two operands
/// happen to render equally — a different op, and not a no-op — never counts.
pub(crate) fn bug387_selfmove_lines(instructions: &[CodeInstruction], target: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for inst in instructions {
        if inst.op != CodeOp::Mov {
            continue;
        }
        if let (Some(dst), Some(src)) = (inst.get("dst"), inst.get("src")) {
            if dst == src {
                lines.push(format!("BUG387-SELFMOVE tgt={target} op=mov reg={dst}"));
            }
        }
    }
    lines
}

/// Remove every `mov <reg>,<reg>` self-move from a finalized instruction stream
/// (plan-85 Phase-D elision). On AArch64/RISC-V the aligned staging moves the
/// shared lowering now emits (`mov return_register(),c_return(0)` after every libc
/// call, and any `%retC`→aligned staging) realize to `mov xN,xN` no-ops, because
/// the argument and result banks coincide (`x0`); this removes exactly those, so
/// those targets stay byte-identical while the SysV-x86 build keeps the real
/// `mov rdi,rax`. Order-preserving; `mov`-only — a store/load whose operands
/// happen to render equally is a different op and is kept (same guard as the
/// probe). Returns the number removed (for a debug/probe cross-check).
pub(crate) fn elide_redundant_self_moves(instructions: &mut Vec<CodeInstruction>) -> usize {
    let before = instructions.len();
    instructions.retain(|inst| {
        if inst.op != CodeOp::Mov {
            return true;
        }
        match (inst.get("dst"), inst.get("src")) {
            (Some(dst), Some(src)) => dst != src,
            _ => true,
        }
    });
    before - instructions.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::abi;

    #[test]
    fn elide_removes_self_moves_keeps_the_rest() {
        // A realized same-register move is dropped; a genuine cross-register move
        // and a same-operand STORE (a different op, not a no-op) are both kept.
        let mut stream = vec![
            abi::move_register("x0", "x0"),
            abi::move_register("x0", "x1"),
            abi::store_u64("x0", "x0", 0),
        ];
        let removed = elide_redundant_self_moves(&mut stream);
        assert_eq!(removed, 1, "exactly the x0,x0 self-move is removed");
        assert_eq!(stream.len(), 2);
        assert_eq!(stream[0].op, CodeOp::Mov); // the x0,x1 move survives
        assert_eq!(stream[0].get("src").as_deref(), Some("x1"));
    }

    #[test]
    fn probe_flags_self_move_and_ignores_distinct() {
        // A realized same-register move (the plan-71-E staging no-op) and a
        // genuine cross-register move in one stream: exactly the first is flagged.
        let stream = vec![
            abi::move_register("x0", "x0"),
            abi::move_register("x0", "x1"),
        ];
        let lines = bug387_selfmove_lines(&stream, "aarch64");
        assert_eq!(
            lines.len(),
            1,
            "exactly the x0,x0 self-move is flagged, not the x0,x1 move: {lines:?}"
        );
        assert!(lines[0].contains("BUG387-SELFMOVE"), "{}", lines[0]);
        assert!(lines[0].contains("tgt=aarch64"), "{}", lines[0]);
        assert!(lines[0].contains("op=mov"), "{}", lines[0]);
        assert!(lines[0].contains("reg=x0"), "{}", lines[0]);
    }

    #[test]
    fn probe_ignores_non_mov_with_equal_operands() {
        // A store whose operands render equally is not a no-op and must not count.
        let stream = vec![abi::store_u64("x0", "x0", 0)];
        let lines = bug387_selfmove_lines(&stream, "riscv64");
        assert!(
            lines.is_empty(),
            "a store with equal operand spellings is not a self-move: {lines:?}"
        );
    }
}
