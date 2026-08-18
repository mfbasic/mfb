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
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;
pub(crate) mod unix;
pub(crate) mod windows;

/// Route `process.__drop` to the Windows (`CreateProcess`) or Unix (fork/exec)
/// backend by `platform.family()`. Every descriptor member now owns its own
/// per-platform dispatch via `Implementation::Os` in its `func_*.rs`; `__drop`
/// is the lone non-member helper still reached by name, so it keeps this shim.
pub(crate) fn lower_process_drop_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    if platform.family() == PlatformFamily::Windows {
        windows::lower_process_drop_helper(symbol, platform_imports, platform)
    } else {
        unix::lower_process_drop_helper(symbol, platform_imports, platform)
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
