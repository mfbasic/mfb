//! Native code generation for the built-in `process` package (plan-90).
//!
//! A `Process` is a native resource (tag 10) sharing the canonical plan-80
//! 96-byte record header — tag@0, handle (the child pid)@8, closed@16, generic
//! STATE@24 — followed by a process-specific tail:
//!
//! ```text
//!   32  stdin-write fd   (parent's write end of the child's stdin; -1 once close'd)
//!   40  stdout-read fd   (parent's read end of the child's stdout)
//!   48  stderr-read fd   (parent's read end of the child's stderr)
//!   56  reaped flag      (0 = child not yet reaped; 1 = reaped, status cached)
//!   64  raw waitpid status (valid when reaped; C's `didSignal` reads WTERMSIG)
//!   72  cached exit code (valid when reaped; waitFor returns it, -1 on signal)
//!   80  stdout read-buffer ptr (sub-plan B; 0 until first read)
//!   88  stderr read-buffer ptr (sub-plan B; 0 until first read)
//! ```
//!
//! Every helper receives the `Process` record pointer in `x0` (the first MFB
//! argument register) and returns the standard `(tag, value)` result in
//! `RESULT_TAG_REGISTER`/`RESULT_VALUE_REGISTER`.
//!
//! Each member now owns its own per-platform emission in its `func_*.rs`
//! (`Implementation::Os`); this module keeps only what is genuinely *shared*
//! across members: the record-tail offset constants below, the reusable `emit_*`
//! builders (`unix`/`windows` submodules), the one `lower_process_send_helper`
//! emitter shared by `send`/`sendBytes`, and the `process.__drop` helper (not a
//! descriptor member, so it is still reached by name).

// --- codegen tier imports (migration) ---
use super::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;

/// The `(instructions, relocations, stack_size)` a `process` OS-seam body emits
/// before the `abi_function` wrapper finalizes it — the successor to the finalized
/// `HelperResult` tuple (see `net`'s `NetBodyParts`). `stack_size` is the sp-relative
/// locals region the body reserves.
pub(crate) type ProcBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `abi_function` body shared by every native `process.*` member (crypto/io/net's
/// clean-room shape). The `abi_function` wrapper seeds the entry label, binds the
/// incoming ABI argument registers, and finalizes; this body dispatches to the
/// family-generic [`lower_process_helper`] by the runtime-call name in
/// [`AbiCtx::call`] and appends its instructions/relocations. All native members
/// register this one body; the aux→primary routing (`spawnEnv`, `sendTimeout`, …)
/// lives in `abi_function_lower` as registry data.
pub(crate) fn lower_process_os_seam(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        lower_process_helper(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    // A `void` location: every process body emits its own fallible ABI, so the
    // wrapper appends no epilogue.
    Ok(ValueResult {
        origin: None,
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: ctx.call.to_string(),
    })
}

/// The family-generic dispatcher for every native `process.*` member (and its
/// `spawnEnv`/`sendTimeout`/… code-form aliases) — reached from the shared
/// [`lower_process_os_seam`] `abi_function` body. It selects the member by the
/// runtime-call name and the posix/win backend by `platform.family()`; each
/// per-member fn returns the pre-finalize [`ProcBodyParts`] the wrapper finalizes.
pub(crate) fn lower_process_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    let win = platform.family() == PlatformFamily::Windows;
    match call {
        "process.spawn" | "process.spawnEnv" => {
            if win {
                func_spawn::lower_process_spawn_helper_win(call, symbol, platform_imports, platform)
            } else {
                func_spawn::lower_process_spawn_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.shell" => {
            if win {
                func_shell::lower_process_shell_helper_win(call, symbol, platform_imports, platform)
            } else {
                func_shell::lower_process_shell_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.pid" => {
            if win {
                func_pid::lower_process_pid_helper_win(call, symbol, platform_imports, platform)
            } else {
                func_pid::lower_process_pid_helper_posix(call, symbol, platform_imports, platform)
            }
        }
        "process.isRunning" => {
            if win {
                func_is_running::lower_process_isrunning_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_is_running::lower_process_isrunning_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.waitFor" => {
            if win {
                func_wait_for::lower_process_waitfor_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_wait_for::lower_process_waitfor_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.close" => {
            if win {
                func_close::lower_process_close_helper_win(call, symbol, platform_imports, platform)
            } else {
                func_close::lower_process_close_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.send" | "process.sendTimeout" => {
            if win {
                func_send::lower_process_send_helper_win(call, symbol, platform_imports, platform)
            } else {
                func_send::lower_process_send_helper_posix(call, symbol, platform_imports, platform)
            }
        }
        "process.sendBytes" | "process.sendBytesTimeout" => {
            if win {
                func_send_bytes::lower_process_send_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_send_bytes::lower_process_send_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.receive" | "process.receiveFrom" => {
            if win {
                func_receive::lower_process_receive_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_receive::lower_process_receive_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.receiveBytes" | "process.receiveBytesFrom" => {
            if win {
                func_receive_bytes::lower_process_receivebytes_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_receive_bytes::lower_process_receivebytes_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.poll" | "process.pollFrom" => {
            if win {
                func_poll::lower_process_poll_helper_win(call, symbol, platform_imports, platform)
            } else {
                func_poll::lower_process_poll_helper_posix(call, symbol, platform_imports, platform)
            }
        }
        "process.signal" => {
            if win {
                func_signal::lower_process_signal_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_signal::lower_process_signal_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.didSignal" => {
            if win {
                func_did_signal::lower_process_didsignal_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_did_signal::lower_process_didsignal_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        "process.detach" => {
            if win {
                func_detach::lower_process_detach_helper_win(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            } else {
                func_detach::lower_process_detach_helper_posix(
                    call,
                    symbol,
                    platform_imports,
                    platform,
                )
            }
        }
        other => Err(format!(
            "native process lowering does not support runtime call '{other}'"
        )),
    }
}

/// Route `process.__drop` to the Windows (`CreateProcess`) or Unix (fork/exec)
/// backend by `platform.family()`. `__drop` is the lone non-member helper still
/// reached by name (the scope-drop op, synthesized during IR lowering, not a
/// descriptor member with a `Body`), so it keeps this self-finalizing shim and its
/// own dispatch arm rather than routing through the `abi_function` path.
pub(crate) fn lower_process_drop_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    if platform.family() == PlatformFamily::Windows {
        gen_windows::lower_process_drop_helper(symbol, platform_imports, platform)
    } else {
        gen_unix::lower_process_drop_helper(symbol, platform_imports, platform)
    }
}

// --- Process record tail (offsets from the record base) ----------------------
pub(crate) const PROC_STDIN_W: usize = 32;
pub(crate) const PROC_STDOUT_R: usize = 40;
pub(crate) const PROC_STDERR_R: usize = 48;
pub(crate) const PROC_REAPED: usize = 56;
pub(crate) const PROC_STATUS: usize = 64;
pub(crate) const PROC_EXITCODE: usize = 72;
// 80 / 88 reserved for sub-plan B's per-fd read buffers.

// The whole tail must fit inside the shared 96-byte envelope (plan-80).
const _: () = assert!(PROC_STDIN_W == 32);
const _: () = assert!(88 + 8 <= RESOURCE_RECORD_SIZE_BYTES);
