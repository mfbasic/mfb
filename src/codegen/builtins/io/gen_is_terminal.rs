//! Shared clean-room codegen seam for the terminal-predicate `AbiFunction` members
//! `io::is{Input,Output,Error}Terminal`.
//!
//! The three members differ only in the probed file descriptor (`fd`) and the
//! result label (`text`), so they all lower through the single [`lower_is_terminal`]
//! here (`func_is_output_terminal`/`func_is_error_terminal`
//! `use super::gen_is_terminal::lower_is_terminal`). Console: `isatty(fd)` folded to
//! `TRUE`/`FALSE` (a failure return is `FALSE`, so the call never raises). App mode:
//! the window is the interactive console, so all three return `TRUE`
//! (`emit_app_io_is_terminal`). Either way the body is appended directly into the
//! member's builder and the `abi_function` wrapper finalizes.

use super::app_unsupported;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Shared `abi_function` body for `io::is{Input,Output,Error}Terminal`, selected by
/// the probed descriptor `fd` and labeled `text`.
pub(crate) fn lower_is_terminal(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    fd: u8,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.build_mode.is_app() {
        // App mode: the window is the interactive console — the platform hook
        // appends `RESULT = TRUE/OK` directly into the builder's vreg stream.
        ctx.platform
            .emit_app_io_is_terminal(&symbol, &mut builder.instructions, &mut builder.relocations)
            .ok_or_else(|| app_unsupported(ctx.platform))??;
    } else {
        // Console: `isatty(fd)` → TRUE when > 0, else FALSE (an error return folds
        // to FALSE, so the predicate never raises).
        let yes = format!("{symbol}_yes");
        let done = format!("{symbol}_done");
        builder.instructions.push(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &fd.to_string(),
        ));
        ctx.platform.emit_is_terminal(
            &symbol,
            ctx.platform_imports,
            &mut builder.instructions,
            &mut builder.relocations,
        )?;
        builder.instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_gt(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::label(&done),
        ]);
        builder.instructions.push(abi::return_());
        builder.stack_size = 16;
    }
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from("void"),
        text: text.to_string(),
    })
}
