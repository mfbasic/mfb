//! Native code generation for the built-in `io` package (plan-72-N migration).
//!
//! `io` is a console/standard-stream package: buffered stdout writes, stderr
//! writes, buffer flush/query/toggle, line/char/byte reads, an input-readiness
//! poll, and the three terminal-detection queries. Every member lowers to an
//! OS-seam runtime helper whose body is arch-neutral `abi::` code branching only on
//! OS family / app-vs-console mode.
//!
//! The 15 members share one family-generic dispatcher, [`lower_io_helper`] — the
//! verbatim `match call` block that lived in `code/mod.rs`'s `lower_runtime_helper`.
//! Each member's `func_*.rs` registers it in *both* the `posix` and `win` slots of a
//! `Body::native_os_seam`; the generic OS-seam dispatch (`crate::codegen::os`)
//! reaches it by `platform.family()`, and the emitter branches internally. `io` has
//! no posix/win difference and no `os_aliases`.
//!
//! `io` consumes the per-compilation [`OsLowerCtx`](crate::codegen::registry::OsLowerCtx):
//! `ctx.build_mode.is_app()` selects the app-transcript vs console path, and
//! `ctx.term_state_offset` carries the TUI shadow-grid routing on `io.print`/`io.write`
//! (plan-35-B) and the cooked-mode restore on `io.readLine`/`io.input` (bug-149).
//!
//! The emitters were the hand-written `lower_io_*_helper` bodies under the former
//! `src/target/shared/code/{io_stdout,io_stdin,io_terminal}.rs`; they are relocated
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
use std::collections::HashMap;
mod stdin;
mod stdout;
mod terminal;

use stdin::*;
use stdout::*;
use terminal::*;
/// Family-generic OS-seam dispatcher for every `io` member — the verbatim `match
/// call` block relocated from `src/target/shared/code/mod.rs`. Registered in both
/// the `posix` and `win` slots of each member's `Body::native_os_seam`. Derives
/// `app_mode`/`term_state_offset` from the per-compilation
/// [`OsLowerCtx`](crate::codegen::registry::OsLowerCtx).
pub(crate) fn lower_io_helper(
    call: &str,
    symbol: &str,
    ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let app_mode = ctx.build_mode.is_app();
    let term_state_offset = ctx.term_state_offset;
    Ok(match call {
        "io.print" | "io.write" | "io.printError" | "io.writeError" => {
            let stderr = matches!(call, "io.printError" | "io.writeError");
            let newline = matches!(call, "io.print" | "io.printError");
            // App mode routes io output to the AppKit transcript window
            // (plan-04-macos-app.md §5.4) instead of a file descriptor.
            if app_mode {
                pad_no_slots(
                    platform
                        .emit_app_io_write_helper(
                            symbol,
                            stderr,
                            newline,
                            term_state_offset,
                            platform_imports,
                        )
                        .ok_or_else(|| {
                            format!(
                                "native target '{}' does not support app-mode io helpers",
                                platform.target()
                            )
                        })??,
                )
            } else {
                lower_io_write_helper(
                    symbol,
                    platform_imports,
                    platform,
                    stderr,
                    newline,
                    term_state_offset,
                )?
            }
        }
        "io.flush" => {
            // App-mode transcript writes are synchronous (each io write blocks on
            // the main thread via performSelectorOnMainThread), so output is
            // already visible; flush succeeds immediately (plan §5.4).
            if app_mode {
                pad_no_slots(platform.emit_app_io_flush_helper(symbol).ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app-mode io helpers",
                        platform.target()
                    )
                })??)
            } else {
                lower_io_flush_helper(symbol, platform_imports, platform)?
            }
        }
        "io.isBuffered" => lower_io_is_buffered_helper(symbol, app_mode)?,
        "io.setBuffered" => lower_io_set_buffered_helper(symbol, app_mode)?,
        "io.pollInput" => lower_io_poll_input_helper(symbol, platform_imports, platform, app_mode)?,
        "io.input" | "io.readLine" => {
            // App-mode io.input writes its prompt to the transcript (via io.write)
            // then reads a line (via io.readLine); io.readLine itself is the
            // unchanged console helper, which reads fd 0 — the window input pipe
            // in app mode (plan §5.4). All other read helpers are likewise
            // unchanged and read the pipe.
            if app_mode && call == "io.input" {
                pad_no_slots(platform.emit_app_io_input_helper(symbol).ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app-mode io helpers",
                        platform.target()
                    )
                })??)
            } else {
                lower_io_read_line_helper(
                    symbol,
                    platform_imports,
                    platform,
                    call == "io.input",
                    app_mode,
                    // bug-149: only a console build that also uses `term::`
                    // brackets the line read with a cooked-mode restore.
                    if app_mode { None } else { term_state_offset },
                )?
            }
        }
        "io.readChar" => lower_io_read_char_helper(symbol, platform_imports, platform, app_mode)?,
        "io.readByte" => lower_io_read_byte_helper(symbol, platform_imports, platform, app_mode)?,
        "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
            let fd = match call {
                "io.isInputTerminal" => 0,
                "io.isOutputTerminal" => 1,
                "io.isErrorTerminal" => 2,
                _ => unreachable!(),
            };
            // App mode: the window is the interactive console, so these return
            // TRUE rather than probing a file descriptor (plan §5.4).
            if app_mode {
                pad_no_slots(
                    platform
                        .emit_app_io_is_terminal_helper(symbol)
                        .ok_or_else(|| {
                            format!(
                                "native target '{}' does not support app-mode io helpers",
                                platform.target()
                            )
                        })??,
                )
            } else {
                lower_io_is_terminal_helper(symbol, platform_imports, platform, fd)?
            }
        }
        // Defensive: unreachable — the runtime-call dispatch only routes a known
        // io.* symbol here. Kept only because matching a `&str` is not exhaustive
        // without a catch-all.
        other => {
            return Err(format!(
                "native code plan does not emit runtime call '{other}'"
            ));
        }
    })
}
