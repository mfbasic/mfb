//! Native code generation for the built-in `app::` presentation-mode helpers
//! (plan-62-B). Both read/write the per-arena presentation-mode word reached off
//! the pinned arena-state register at `presentation_mode_offset` (reserved one
//! slot past the `term::` state region, app builds only).
//!
//! `getMode` loads the word into the result value register — the `term::isOn`
//! shape. `setMode` stores its argument, then calls the per-backend
//! surface-reconcile seam (`CodegenPlatform::emit_app_mode_reconcile`): a no-op in
//! B (state-only), filled by plan-62-C (macOS AppKit) and plan-62-D (GTK4) with
//! the real window teardown/rebuild marshalled to the UI thread.

use super::*;
use crate::target::shared::abi;

pub(super) fn lower_app_helper(
    call: &str,
    symbol: &str,
    presentation_mode_offset: usize,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    match call {
        "app.getMode" => emit_get_mode(presentation_mode_offset, &mut instructions),
        "app.setMode" => emit_set_mode(
            symbol,
            presentation_mode_offset,
            platform,
            &mut instructions,
            &mut relocations,
        )?,
        other => return Err(format!("unknown app runtime helper '{other}'")),
    }

    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 0);
    Ok((frame, instructions, relocations, stack_slots))
}

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
pub(super) fn prepend_wrong_mode_gate(
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    symbol: &str,
    presentation_mode_offset: Option<usize>,
    function_id: &str,
) {
    let Some(offset) = presentation_mode_offset else {
        return;
    };
    let _builtin = crate::builtins::descriptor::REGISTRY.function(function_id);
    let ok = format!("{symbol}_mode_ok");
    let mut gate = vec![
        abi::load_u64(abi::SCRATCH[0], ARENA_STATE_REGISTER, offset),
        abi::compare_immediate(abi::SCRATCH[0], "0"), // Console == 0
        abi::branch_eq(&ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_WRONG_MODE_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ];
    push_error_message_address(symbol, ERR_WRONG_MODE_SYMBOL, &mut gate, relocations);
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

/// `app::getMode()` — load the presentation-mode word (`0`/`1`) into the result
/// value register as a `Mode` value (the enum is i64-carried by its discriminant).
fn emit_get_mode(presentation_mode_offset: usize, instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        presentation_mode_offset,
    ));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `app::setMode(mode)` — store the `Mode` discriminant into the presentation-mode
/// word, then invoke the per-backend surface-reconcile seam. The store lands in
/// memory *before* the reconcile runs, so the reconcile (which in C/D emits
/// register-clobbering `bl` calls to marshal to the UI thread) reads the
/// authoritative mode from the slot rather than a caller-saved register. Returns
/// Nothing.
fn emit_set_mode(
    symbol: &str,
    presentation_mode_offset: usize,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.push(abi::move_register("%v9", abi::ARG[0]));
    instructions.push(abi::store_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        presentation_mode_offset,
    ));
    // plan-62-B seam (no-op default; filled by plan-62-C/D). `None` = state-only.
    if let Some(result) =
        platform.emit_app_mode_reconcile(symbol, presentation_mode_offset, instructions, relocations)
    {
        result?;
    }
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    Ok(())
}
