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
//! `RESULT_TAG_REGISTER`/`RESULT_VALUE_REGISTER`. The Unix mechanism (fork/exec,
//! three pipes, waitpid/kill) lives in the `unix` submodule; the Windows backend
//! (`CreateProcess`) is added by sub-plan D.

use crate::target::shared::code::*;
use std::collections::HashMap;

mod unix;
mod windows;

/// Route a process helper to the Windows (`CreateProcess`) or Unix (fork/exec)
/// backend by `platform.family()`. plan-90-D adds the Windows arm; on every other
/// platform this is a thin pass-through to `unix`.
macro_rules! process_dispatch {
    ($name:ident ( $($arg:ident : $ty:ty),* $(,)? )) => {
        pub(crate) fn $name(
            symbol: &str,
            platform_imports: &HashMap<String, String>,
            platform: &dyn CodegenPlatform,
            $($arg : $ty),*
        ) -> HelperResult {
            if platform.family() == PlatformFamily::Windows {
                windows::$name(symbol, platform_imports, platform $(, $arg)*)
            } else {
                unix::$name(symbol, platform_imports, platform $(, $arg)*)
            }
        }
    };
}

process_dispatch!(lower_process_spawn_helper(with_env: bool));
process_dispatch!(lower_process_spawnenv_helper());
process_dispatch!(lower_process_shell_helper());
process_dispatch!(lower_process_pid_helper());
process_dispatch!(lower_process_isrunning_helper());
process_dispatch!(lower_process_waitfor_helper());
process_dispatch!(lower_process_close_helper());
process_dispatch!(lower_process_send_helper(is_bytes: bool, with_timeout: bool));
process_dispatch!(lower_process_receive_helper(with_from: bool));
process_dispatch!(lower_process_receivebytes_helper(with_from: bool));
process_dispatch!(lower_process_poll_helper(with_from: bool));
process_dispatch!(lower_process_signal_helper());
process_dispatch!(lower_process_didsignal_helper());
process_dispatch!(lower_process_detach_helper());
process_dispatch!(lower_process_drop_helper());

// --- Process record tail (offsets from the record base) ----------------------
pub(super) const PROC_STDIN_W: usize = 32;
pub(super) const PROC_STDOUT_R: usize = 40;
pub(super) const PROC_STDERR_R: usize = 48;
pub(super) const PROC_REAPED: usize = 56;
pub(super) const PROC_STATUS: usize = 64;
pub(super) const PROC_EXITCODE: usize = 72;
// 80 / 88 reserved for sub-plan B's per-fd read buffers.

// The whole tail must fit inside the shared 96-byte envelope (plan-80).
const _: () = assert!(PROC_STDIN_W == 32);
const _: () = assert!(88 + 8 <= RESOURCE_RECORD_SIZE_BYTES);
