//! Native code generation for the built-in `io` package (plan-72-N migration).
//!
//! `io` is a console/standard-stream package: buffered stdout writes, stderr
//! writes, buffer flush/query/toggle, line/char/byte reads, an input-readiness
//! poll, and the three terminal-detection queries. Every member lowers to an
//! OS-seam runtime helper whose body is arch-neutral `abi::` code branching only on
//! OS family / app-vs-console mode.
//!
//! Each member is a per-function `Body::abi_function` clean-room lowering
//! (plan-101): its `func_*.rs` `lower_*` adapter calls the matching
//! `lower_io_*_helper` (console) or platform app hook (app mode) here and hands
//! the finished body back through the pre-finalized hatch. This module owns the
//! shared emitters and the small adapter glue ([`hatch_finalized`],
//! [`adapter_app_mode`], [`app_unsupported`], [`lower_write_family`],
//! [`lower_read_line_family`], [`lower_is_terminal_common`]); the former
//! family-generic `lower_io_helper` `match call` dispatcher — one `Body::native_os_seam`
//! slot per member — is retired. `io` has no posix/win difference and no `os_aliases`.
//!
//! `io` consumes its OS-seam context through the threaded
//! [`AbiCtx`](crate::codegen::registry::AbiCtx): `ctx.build_mode.is_app()` selects
//! the app-transcript vs console path, and `ctx.term_state_offset` carries the TUI
//! shadow-grid routing on `io.print`/`io.write` (plan-35-B) and the cooked-mode
//! restore on `io.readLine`/`io.input` (bug-149).
//!
//! The emitters were the hand-written `lower_io_*_helper` bodies under the former
//! `src/codegen/io/{stdout,stdin,terminal}`; they are relocated
//! here verbatim (byte-identical emission). Shared primitives they call
//! (`emit_append_to_buffer`, `TerminalModeSlots`, `emit_stdin_next_byte`,
//! `emit_configure_stdin_terminal`, …) stay in the shared code layer because other
//! packages use them, reachable here through the `code::*` glob.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::stdin::*;
use crate::codegen::io::stdout::*;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
mod stdin;
mod stdout;
mod terminal;

// `pub(crate)` re-export so each `func_*.rs` abi_function adapter (plan-101) can
// reach its emitter as `super::native::lower_io_*_helper`.
pub(crate) use stdin::*;
pub(crate) use stdout::*;
pub(crate) use terminal::*;

use crate::codegen::engine::operand::Operand;
use crate::codegen::registry::AbiCtx;

/// Hand an already-finalized OS-seam helper body back to `lower_abi_function_helper`
/// through the plan-101 pre-finalized hatch: place the finished stream in
/// `builder.instructions`/`relocations`, stash `(frame, slots)` in
/// `builder.abi_prefinalized`, and return the `void`-location `ValueResult` an
/// abi_function body signals completion with. Shared by every migrated `io`
/// abi_function adapter — the body an emitter (or an app-mode `AppHookBody`)
/// produced is byte-identical to what `lower_io_helper` returned pre-migration.
pub(crate) fn hatch_finalized(
    builder: &mut CodeBuilder,
    body: HelperBody,
    return_type: &str,
    text: &str,
) -> Result<ValueResult, String> {
    let (frame, instructions, relocations, stack_slots) = body;
    builder.instructions = instructions;
    builder.relocations = relocations;
    builder.abi_prefinalized = Some((frame, stack_slots));
    Ok(ValueResult {
        type_: return_type.to_string(),
        location: Operand::from("void"),
        text: text.to_string(),
    })
}

/// The app-vs-console mode an io abi_function adapter branches on, read from the
/// `AbiCtx` the plan-101 dispatch threads in.
pub(crate) fn adapter_app_mode(ctx: &AbiCtx) -> bool {
    ctx.build_mode.is_app()
}

/// The error a migrated io abi_function adapter raises when the target lacks an
/// app-mode io hook — the verbatim message the former `lower_io_helper` produced.
pub(crate) fn app_unsupported(platform: &dyn CodegenPlatform) -> String {
    format!(
        "native target '{}' does not support app-mode io helpers",
        platform.target()
    )
}

/// Shared `abi_function` body for the two line readers `io::input` (with a
/// prompt) and `io::readLine` (no prompt). App-mode `io::input` writes its prompt
/// to the transcript then reads a line (`emit_app_io_input_helper`); every other
/// case — console input/readLine, and app-mode readLine — is the shared console
/// reader (`lower_io_read_line_helper`), which reads fd 0 (the window input pipe
/// in app mode). `console_term_state` is `None` in app mode (no tty) and the
/// threaded `term_state_offset` in a console build (bug-149 cooked-mode restore).
/// Hatched back pre-finalized.
pub(crate) fn lower_read_line_family(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    with_prompt: bool,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let app = adapter_app_mode(ctx);
    let body = if app && with_prompt {
        pad_no_slots(
            ctx.platform
                .emit_app_io_input_helper(&symbol)
                .ok_or_else(|| app_unsupported(ctx.platform))??,
        )
    } else {
        lower_io_read_line_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            with_prompt,
            app,
            if app { None } else { ctx.term_state_offset },
        )?
    };
    hatch_finalized(builder, body, "String", text)
}

/// Shared `abi_function` body for the four stdout writers
/// `io::{print,write,printError,writeError}`, which differ only in target stream
/// (`stderr`) and whether a trailing newline is appended (`newline`). Console:
/// `lower_io_write_helper` (loops the `write(fd, …)`, TUI-shadow-grid-routed while
/// `term::` is active). App mode: the transcript-window write hook
/// (`emit_app_io_write_helper`). The string/attributed-string overloads share
/// this one helper (both pass a string-object pointer in arg 0), exactly as the
/// pre-migration `native_os_seam` slot did. Hatched back pre-finalized.
pub(crate) fn lower_write_family(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    stderr: bool,
    newline: bool,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let body = if adapter_app_mode(ctx) {
        pad_no_slots(
            ctx.platform
                .emit_app_io_write_helper(
                    &symbol,
                    stderr,
                    newline,
                    ctx.term_state_offset,
                    ctx.platform_imports,
                )
                .ok_or_else(|| app_unsupported(ctx.platform))??,
        )
    } else {
        lower_io_write_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            stderr,
            newline,
            ctx.term_state_offset,
        )?
    };
    hatch_finalized(builder, body, "Nothing", text)
}

/// Shared `abi_function` body for the three terminal predicates
/// `io::is{Input,Output,Error}Terminal`, which differ only in the probed file
/// descriptor (`fd`) and the result label (`text`). Console: `isatty(fd)` via
/// `lower_io_is_terminal_helper`. App mode: the window is the interactive
/// console, so all three return `TRUE` (`emit_app_io_is_terminal_helper`).
pub(crate) fn lower_is_terminal_common(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    fd: u8,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let body = if adapter_app_mode(ctx) {
        pad_no_slots(
            ctx.platform
                .emit_app_io_is_terminal_helper(&symbol)
                .ok_or_else(|| app_unsupported(ctx.platform))??,
        )
    } else {
        lower_io_is_terminal_helper(&symbol, ctx.platform_imports, ctx.platform, fd)?
    };
    hatch_finalized(builder, body, "Boolean", text)
}
