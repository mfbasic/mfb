//! Windows (`win_x86_64`) native backend for the `process` package (plan-90-D).
//!
//! Reimplements the same `process` surface over Win32 — `CreateProcessA` +
//! anonymous pipes + `WaitForSingleObject`/`GetExitCodeProcess` +
//! `TerminateProcess` — sharing the same tag-10 record and 96-byte envelope. The
//! record tail reuses the Unix slot offsets (`PROC_STDIN_W`.. from `process/mod`),
//! now holding Win32 `HANDLE`s (64-bit, they fit) instead of fds; the handle word
//! (`RESOURCE_OFFSET_HANDLE`@8) holds the process `HANDLE` and the pid lives in
//! `PROC_STATUS`-adjacent tail slots as needed.
//!
//! This backend is landed in phases, gated by the `win_x86_64` capability list
//! (like the rest of that backend, which "advertises a minimal capability set"):
//! a function whose capability is not yet advertised never reaches its helper, so
//! the not-yet-emitted arms below are unreachable placeholders, not live stubs.

use super::*;
use std::collections::HashMap;

fn unimplemented_on_windows(op: &str) -> HelperResult {
    // Unreachable: the win_x86_64 capability list does not advertise this call
    // until its Win32 emission lands, so validation rejects it before codegen.
    Err(format!(
        "process::{op} native Windows backend is not yet emitted (plan-90-D)"
    ))
}

pub(in crate::target::shared::code) fn lower_process_spawn_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
    _with_env: bool,
) -> HelperResult {
    unimplemented_on_windows("spawn")
}

pub(in crate::target::shared::code) fn lower_process_spawnenv_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("spawn")
}

pub(in crate::target::shared::code) fn lower_process_shell_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("shell")
}

pub(in crate::target::shared::code) fn lower_process_pid_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("pid")
}

pub(in crate::target::shared::code) fn lower_process_isrunning_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("isRunning")
}

pub(in crate::target::shared::code) fn lower_process_waitfor_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("waitFor")
}

pub(in crate::target::shared::code) fn lower_process_close_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("close")
}

pub(in crate::target::shared::code) fn lower_process_send_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
    _is_bytes: bool,
    _with_timeout: bool,
) -> HelperResult {
    unimplemented_on_windows("send")
}

pub(in crate::target::shared::code) fn lower_process_receive_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
    _with_from: bool,
) -> HelperResult {
    unimplemented_on_windows("receive")
}

pub(in crate::target::shared::code) fn lower_process_receivebytes_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
    _with_from: bool,
) -> HelperResult {
    unimplemented_on_windows("receiveBytes")
}

pub(in crate::target::shared::code) fn lower_process_poll_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
    _with_from: bool,
) -> HelperResult {
    unimplemented_on_windows("poll")
}

pub(in crate::target::shared::code) fn lower_process_signal_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("signal")
}

pub(in crate::target::shared::code) fn lower_process_didsignal_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("didSignal")
}

pub(in crate::target::shared::code) fn lower_process_detach_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("detach")
}

pub(in crate::target::shared::code) fn lower_process_drop_helper(
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("__drop")
}
