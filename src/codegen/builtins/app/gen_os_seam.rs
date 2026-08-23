//! Native code generation for the built-in `app::` presentation-mode helpers
//! (plan-62-B), relocated onto the clean-room registry.
//!
//! Both members read/write the per-arena presentation-mode word reached off the
//! pinned arena-state register at `presentation_mode_offset` (reserved one slot
//! past the `term::` state region, app builds only). The offset is not a static
//! ABI constant — it is a per-compilation `ArenaLayout` value — so it arrives on
//! the [`AbiCtx`](crate::codegen::registry::AbiCtx) the `abi_function` wrapper
//! threads to the shared [`lower_app_os_seam`] body; `presentation_mode_offset` is
//! `Some` exactly in an `--app` build, so its absence here is an internal error
//! (the plan never emits these symbols otherwise).
//!
//! `getMode` loads the word into the result value register — the `term::isOn`
//! shape. `setMode` stores its argument, then calls the per-backend
//! surface-reconcile seam (`CodegenPlatform::emit_app_mode_reconcile`): a no-op in
//! B (state-only), filled by plan-62-C (macOS AppKit) and plan-62-D (GTK4) with
//! the real window teardown/rebuild marshalled to the UI thread.
//!
//! The cross-package `ErrWrongMode` gate that fences `term::` / console-read `io::`
//! helpers outside `Console` mode is NOT part of this package — it stays in the
//! shared code layer (`src/codegen/app/hook/app.rs::prepend_wrong_mode_gate`),
//! since it splices into other packages' helper bodies.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;

use crate::codegen::engine::operand::*;
use crate::target::shared::abi;

/// The `(instructions, relocations, stack_size)` an `app` OS-seam body emits before
/// the `abi_function` wrapper finalizes it (see `net`'s `NetBodyParts`).
pub(crate) type AppBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `abi_function` body shared by the two `app::` presentation-mode members
/// (crypto/io/net's clean-room shape). The `abi_function` wrapper seeds the entry
/// label, binds the ABI argument registers, and finalizes; this body dispatches to
/// [`lower_app_helper`] by the runtime-call name in [`AbiCtx::call`] and appends its
/// instructions/relocations.
pub(crate) fn lower_app_os_seam(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = lower_app_helper(
        ctx.call,
        &symbol,
        ctx.presentation_mode_offset,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(ValueResult {
        origin: None,
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: ctx.call.to_string(),
    })
}

/// The two `app::` presentation-mode runtime helpers. Reads the per-arena
/// presentation-mode slot (`getMode`) or writes it and runs the per-backend
/// surface-reconcile seam (`setMode`), returning the pre-finalize [`AppBodyParts`]
/// the wrapper finalizes. `presentation_mode_offset` is the arena slot; it is `Some`
/// only in an `--app` build, so `None` here is an internal error.
pub(crate) fn lower_app_helper(
    call: &str,
    symbol: &str,
    presentation_mode_offset: Option<usize>,
    platform: &dyn CodegenPlatform,
) -> Result<AppBodyParts, String> {
    let presentation_mode_offset = presentation_mode_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving the presentation-mode slot")
    })?;

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();

    match call {
        "app.getMode" => emit_get_mode(presentation_mode_offset, &mut instructions),
        "app.setMode" => emit_set_mode(
            symbol,
            presentation_mode_offset,
            platform,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        )?,
        other => return Err(format!("unknown app runtime helper '{other}'")),
    }

    instructions.push(abi::return_());
    Ok((instructions, relocations, 0))
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
    vregs: &mut Vregs,
) -> Result<(), String> {
    let mode = vregs.next();
    instructions.push(abi::move_register(&mode, abi::c_arg(0)));
    instructions.push(abi::store_u64(
        &mode,
        ARENA_STATE_REGISTER,
        presentation_mode_offset,
    ));
    // plan-62-B seam (no-op default; filled by plan-62-C/D). `None` = state-only.
    if let Some(result) = platform.emit_app_mode_reconcile(
        symbol,
        presentation_mode_offset,
        instructions,
        relocations,
    ) {
        result?;
    }
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    Ok(())
}
