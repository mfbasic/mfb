//! The cross-package `ErrWrongMode` presentation-mode gate (plan-62-E).
//!
//! The `app::` presentation mode fences the `term::` and console-read `io::`
//! helpers: outside `Console` mode those surfaces do not exist, so an ungated call
//! must raise the trappable `ErrWrongMode` rather than touch (or block on) an
//! absent grid/input pipe. This gate splices that early error return into *other
//! packages'* helper bodies, so it lives in the shared code layer rather than in
//! the migrated `app` package (which owns only the `Mode` enum and the
//! `getMode`/`setMode` members — those lower through
//! `codegen::builtins::app::native`).
//!
//! The gated helpers are emitted with the arena `presentation_mode_offset`; when it
//! is `None` (the program never uses `app::`, so it can never leave `Console`),
//! nothing is spliced and the body is byte-identical to a non-`app::` program's.

// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
/// plan-62-E: prepend an `ErrWrongMode` gate to an app-mode `term::` / console-read
/// `io::` helper body. When `presentation_mode_offset` is `Some` (the program uses
/// `app::`, so a non-`Console` mode is reachable), the gate loads the presentation
/// mode and, if it is not `Console` (`0`), raises the trappable `ErrWrongMode`
/// before the helper does any work; otherwise it falls through. When `None` (the
/// program can never leave `Console`), nothing is emitted, so a non-`app::` program
/// is byte-identical.
///
/// The gate is spliced in right after the helper's `"entry"` label and *before* its
/// manual prologue (`subtract_stack`), so the early error return runs with no frame
/// allocated and an intact link register — a bare `return_()` is safe there.
pub(crate) fn prepend_wrong_mode_gate(
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    symbol: &str,
    presentation_mode_offset: Option<usize>,
) {
    let Some(offset) = presentation_mode_offset else {
        return;
    };
    let ok = format!("{symbol}_mode_ok");
    let mut gate = vec![
        abi::load_u64(abi::SCRATCH[0], ARENA_STATE_REGISTER, offset),
        abi::compare_immediate(abi::SCRATCH[0], "0"), // Console == 0
        abi::branch_eq(&ok),
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

    #[test]
    fn absent_presentation_state_leaves_the_helper_byte_identical() {
        let mut instructions = vec![abi::label("entry"), abi::return_()];
        let mut relocations = Vec::new();

        prepend_wrong_mode_gate(&mut instructions, &mut relocations, "#io_input", None);

        assert_eq!(instructions.len(), 2);
        assert_eq!(instructions[0].op, CodeOp::Label);
        assert_eq!(instructions[1].op, CodeOp::Ret);
        assert!(relocations.is_empty());
    }

    #[test]
    fn wrong_mode_gate_is_inserted_after_an_entry_label() {
        let mut instructions = vec![abi::label("entry"), abi::return_()];
        let mut relocations = Vec::new();

        prepend_wrong_mode_gate(&mut instructions, &mut relocations, "#io_input", Some(40));

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

        prepend_wrong_mode_gate(&mut instructions, &mut relocations, "#term_on", Some(8));

        assert_eq!(instructions.first().unwrap().op, CodeOp::LdrU64);
        assert_eq!(instructions.last().unwrap().op, CodeOp::Ret);
    }
}
