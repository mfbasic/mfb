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
use crate::types::ParameterType;
use std::collections::HashMap;

/// The `(instructions, relocations, stack_size)` a `process` OS-seam body emits
/// before the `abi_function` wrapper finalizes it — the successor to the finalized
/// `HelperResult` tuple (see `net`'s `NetBodyParts`). `stack_size` is the sp-relative
/// locals region the body reserves.
pub(crate) type ProcBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `void` result every native `process.*` member returns from its per-member
/// `abi_function` body: every process body emits its own fallible ABI, so the wrapper
/// appends no epilogue. `type_` is `Nothing`; `text` carries the runtime-call name.
pub(crate) fn void_result(call: &str) -> ValueResult {
    ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: call.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::engine::tests::test_support::TestPlatform;

    #[test]
    fn void_result_preserves_the_runtime_call_identity() {
        let result = void_result("process.close");
        assert!(result.origin.is_none());
        assert_eq!(result.type_, ParameterType::Nothing);
        assert_eq!(result.location, Operand::from("void"));
        assert_eq!(result.text, "process.close");
    }

    #[test]
    fn process_drop_dispatches_to_unix_and_windows_emitters() {
        let mut imports = HashMap::new();
        imports.insert("TerminateProcess".to_string(), "kernel32".to_string());
        imports.insert("CloseHandle".to_string(), "kernel32".to_string());
        crate::codegen::engine::mir::set_backend(TestPlatform.backend());
        let (_, unix, _, _) = lower_process_drop_helper("#process_drop", &imports, &TestPlatform)
            .expect("unix process drop lowers");
        let windows_platform = crate::target::win_x86_64::code::Platform;
        crate::codegen::engine::mir::set_backend(windows_platform.backend());
        let (_, windows, _, _) =
            lower_process_drop_helper("#process_drop", &imports, &windows_platform)
                .expect("windows process drop lowers");

        assert!(unix
            .iter()
            .any(|instruction| instruction.op == crate::arch::ops::CodeOp::BranchLink));
        assert!(windows
            .iter()
            .any(|instruction| instruction.op == crate::arch::ops::CodeOp::BranchLink));
        assert_ne!(unix.len(), windows.len());
    }
}
