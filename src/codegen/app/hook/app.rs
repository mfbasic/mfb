//! The cross-package `ErrWrongMode` presentation-mode gate (plan-62-E, plan-98-A).
//!
//! The `app::` presentation mode fences the `term::` and console-read `io::`
//! helpers: in a mode where the surface they need does not exist, an ungated call
//! must raise the trappable `ErrWrongMode` rather than touch (or block on) an
//! absent grid/input pipe. This gate splices that early error return into *other
//! packages'* helper bodies, so it lives in the shared code layer rather than in
//! the migrated `app` package (which owns only the `Mode` enum and the
//! `getMode`/`setMode` members — those lower through
//! `codegen::builtins::app::native`).
//!
//! **The two gated families need different predicates** (plan-98-A Phase 2), which
//! is why [`ModeRequirement`] exists:
//!
//! - `term::` needs the *character grid*, which only the `Console` transcript view
//!   has. It stays `Console`-only: `None` and `Canvas` both trap.
//! - The console-read `io::` helpers need only *an input source*. `Console` has one
//!   (the transcript view's key events) and so does `Canvas` (the canvas window's
//!   key events, wired in Phase 4) — they read the same fd-0 pipe. Only `None`, which
//!   has no window at all, has nowhere for input to come from. So those helpers trap
//!   in `None` alone.
//!
//! The gated helpers are emitted with the arena `presentation_mode_offset`; when it
//! is `None` (the program never uses `app::`, so it can never leave `Console`),
//! nothing is spliced and the body is identical to a non-`app::` program's.

// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
/// Which presentation modes a gated helper is allowed to run in.
///
/// The discriminants the gate compares against are the `Mode` enum's, fixed by
/// variant declaration order in `src/codegen/builtins/app/mod.rs:register`:
/// `Console` = 0, `None` = 1, `Canvas` = 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModeRequirement {
    /// `Console` only — the helper needs the transcript view's character grid, which
    /// no other mode has. Emitted as "fall through iff mode `== 0`". Used by every
    /// app-mode `term::` helper.
    Console,
    /// Any mode with a window, i.e. anything but `None` — the helper needs only an
    /// input source, and `Console` and `Canvas` both deliver their window's key
    /// events into the same fd-0 pipe. Emitted as "fall through iff mode `!= 1`".
    /// Used by the console-read `io::` helpers.
    ///
    /// Deliberately expressed as "not `None`" rather than "`Console` or `Canvas`":
    /// a future windowed mode gets an input source for free, and — more importantly —
    /// the alternative would be a two-compare gate whose second arm a new variant
    /// would silently fall off the end of, trapping in a mode that has a window.
    WindowedMode,
}

/// plan-62-E / plan-98-A: prepend an `ErrWrongMode` gate to an app-mode `term::` /
/// console-read `io::` helper body. When `presentation_mode_offset` is `Some` (the
/// program uses `app::`, so a non-`Console` mode is reachable), the gate loads the
/// presentation mode and, if `requirement` is not satisfied, raises the trappable
/// `ErrWrongMode` before the helper does any work; otherwise it falls through. When
/// `None` (the program can never leave `Console`), nothing is emitted, so a
/// non-`app::` program is unchanged.
///
/// The gate is spliced in right after the helper's `"entry"` label and *before* its
/// manual prologue (`subtract_stack`), so the early error return runs with no frame
/// allocated and an intact link register — a bare `return_()` is safe there.
pub(crate) fn prepend_wrong_mode_gate(
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    symbol: &str,
    presentation_mode_offset: Option<usize>,
    requirement: ModeRequirement,
) {
    let Some(offset) = presentation_mode_offset else {
        return;
    };
    let ok = format!("{symbol}_mode_ok");
    // One compare either way — the two requirements differ only in the immediate and
    // the branch condition, so neither costs more than the other on the hot path.
    let (immediate, branch) = match requirement {
        ModeRequirement::Console => ("0", abi::branch_eq(&ok)), // fall through iff Console
        ModeRequirement::WindowedMode => ("1", abi::branch_ne(&ok)), // trap iff None
    };
    let mut gate = vec![
        abi::load_u64(abi::SCRATCH[0], ARENA_STATE_REGISTER, offset),
        abi::compare_immediate(abi::SCRATCH[0], immediate),
        branch,
    ];
    raise_error_into(symbol, "ErrWrongMode", &mut gate, relocations);
    gate.push(abi::return_());
    gate.push(abi::label(&ok));
    // Splice after the `"entry"` label (index 0), before the manual prologue.
    let at = if instructions
        .first()
        .is_some_and(|instruction| instruction.op == CodeOp::Label)
    {
        1
    } else {
        0
    };
    instructions.splice(at..at, gate);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compare immediate and the branch condition the gate emitted, so the two
    /// requirements can be told apart by what they actually encode rather than by
    /// re-asserting the constructor argument.
    fn gate_predicate(instructions: &[CodeInstruction]) -> (String, CodeOp) {
        let compare = instructions
            .iter()
            .find(|instruction| instruction.op == CodeOp::CmpImm)
            .expect("gate emits a compare-immediate");
        let immediate = compare
            .fields
            .iter()
            .find(|(name, _)| *name == "rhs")
            .map(|(_, value)| value.to_string())
            .expect("compare_immediate names its immediate `rhs`");
        let branch = instructions
            .iter()
            .find(|instruction| matches!(instruction.op, CodeOp::BranchEq | CodeOp::BranchNe))
            .expect("gate emits a conditional branch")
            .op;
        (immediate, branch)
    }

    #[test]
    fn absent_presentation_state_leaves_the_helper_byte_identical() {
        for requirement in [ModeRequirement::Console, ModeRequirement::WindowedMode] {
            let mut instructions = vec![abi::label("entry"), abi::return_()];
            let mut relocations = Vec::new();

            prepend_wrong_mode_gate(
                &mut instructions,
                &mut relocations,
                "#io_input",
                None,
                requirement,
            );

            assert_eq!(instructions.len(), 2, "{requirement:?}");
            assert_eq!(instructions[0].op, CodeOp::Label);
            assert_eq!(instructions[1].op, CodeOp::Ret);
            assert!(relocations.is_empty(), "{requirement:?}");
        }
    }

    #[test]
    fn wrong_mode_gate_is_inserted_after_an_entry_label() {
        let mut instructions = vec![abi::label("entry"), abi::return_()];
        let mut relocations = Vec::new();

        prepend_wrong_mode_gate(
            &mut instructions,
            &mut relocations,
            "#io_input",
            Some(40),
            ModeRequirement::WindowedMode,
        );

        assert_eq!(instructions.first().unwrap().op, CodeOp::Label);
        assert_eq!(instructions[1].op, CodeOp::LdrU64);
        assert_eq!(instructions.last().unwrap().op, CodeOp::Ret);
        assert!(instructions.iter().any(|instruction| {
            instruction.op == CodeOp::Label
                && instruction
                    .fields
                    .iter()
                    .any(|(name, value)| *name == "name" && value == "#io_input_mode_ok")
        }));
        assert!(!relocations.is_empty());
    }

    #[test]
    fn wrong_mode_gate_prepends_a_body_without_an_entry_label() {
        let mut instructions = vec![abi::return_()];
        let mut relocations = Vec::new();

        prepend_wrong_mode_gate(
            &mut instructions,
            &mut relocations,
            "#term_on",
            Some(8),
            ModeRequirement::Console,
        );

        assert_eq!(instructions.first().unwrap().op, CodeOp::LdrU64);
        assert_eq!(instructions.last().unwrap().op, CodeOp::Ret);
    }

    /// plan-98-A Phase 2: `term::` keeps the `Console`-only predicate — "fall through
    /// iff mode == 0", so both `None` (1) and `Canvas` (2) trap.
    #[test]
    fn console_requirement_falls_through_only_on_console() {
        let mut instructions = vec![abi::label("entry"), abi::return_()];
        let mut relocations = Vec::new();

        prepend_wrong_mode_gate(
            &mut instructions,
            &mut relocations,
            "#term_on",
            Some(8),
            ModeRequirement::Console,
        );

        let (immediate, branch) = gate_predicate(&instructions);
        assert_eq!(immediate, "0", "compares against Console == 0");
        assert_eq!(
            branch,
            CodeOp::BranchEq,
            "falls through on equality, so 1 (None) and 2 (Canvas) both trap"
        );
    }

    /// plan-98-A Phase 2: the console-read `io::` helpers trap **only** in `None`, so
    /// `Canvas` (2) reads the canvas window's keys exactly as `Console` (0) reads the
    /// transcript view's. Expressed as a single `!= 1` compare, so any future windowed
    /// mode inherits the input source instead of silently trapping.
    #[test]
    fn windowed_requirement_traps_only_on_none() {
        let mut instructions = vec![abi::label("entry"), abi::return_()];
        let mut relocations = Vec::new();

        prepend_wrong_mode_gate(
            &mut instructions,
            &mut relocations,
            "#io_read_line",
            Some(8),
            ModeRequirement::WindowedMode,
        );

        let (immediate, branch) = gate_predicate(&instructions);
        assert_eq!(immediate, "1", "compares against None == 1");
        assert_eq!(
            branch,
            CodeOp::BranchNe,
            "falls through on inequality, so only 1 (None) traps"
        );
    }

    /// The two requirements must not emit the same gate — if they did, the Phase 2
    /// relaxation would be a no-op that every behavioral test above still passed.
    #[test]
    fn the_two_requirements_emit_different_gates() {
        let build = |requirement| {
            let mut instructions = vec![abi::label("entry"), abi::return_()];
            let mut relocations = Vec::new();
            prepend_wrong_mode_gate(
                &mut instructions,
                &mut relocations,
                "#gate",
                Some(8),
                requirement,
            );
            gate_predicate(&instructions)
        };
        assert_ne!(
            build(ModeRequirement::Console),
            build(ModeRequirement::WindowedMode)
        );
    }
}
