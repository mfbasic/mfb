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
//!   80  stdout spill-buffer ptr (bug-475; 0 until `waitFor` has to drain)
//!   88  stderr spill-buffer ptr (bug-475; 0 until `waitFor` has to drain)
//! ```
//!
//! The two spill buffers are what keeps `waitFor` from deadlocking against a
//! child that outruns the pipe (bug-475). While it waits, `waitFor` moves
//! whatever the child has written out of the kernel pipe and into an arena block
//! hanging off slot 80/88; `receive`/`receiveBytes`/`poll` serve from that block
//! first and only then fall through to the fd, so the output is delivered in
//! order and nothing is discarded. Each block is:
//!
//! ```text
//!    0  capacity (bytes of the data region)
//!    8  length   (bytes written by the drain)
//!   16  offset   (bytes already handed back to a reader)
//!   24  data ...
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
use crate::target::shared::abi;
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
pub(crate) const PROC_STDOUT_BUF: usize = 80;
pub(crate) const PROC_STDERR_BUF: usize = 88;

// The whole tail must fit inside the shared 96-byte envelope (plan-80).
const _: () = assert!(PROC_STDIN_W == 32);
const _: () = assert!(PROC_STDERR_BUF + 8 <= RESOURCE_RECORD_SIZE_BYTES);

// --- Spill-buffer block (bug-475) --------------------------------------------
/// Bytes of the data region this block can hold.
pub(crate) const SPILL_CAPACITY: usize = 0;
/// Bytes the drain has written into the data region.
pub(crate) const SPILL_LENGTH: usize = 8;
/// Bytes of the data region a reader has already consumed.
pub(crate) const SPILL_OFFSET: usize = 16;
/// Start of the data region.
pub(crate) const SPILL_DATA: usize = 24;
/// The first block's data capacity. Doubles from here as the drain fills it.
pub(crate) const SPILL_INITIAL_CAPACITY: usize = 65536;
/// The most one drain `read` moves out of the pipe in a single pass.
pub(crate) const SPILL_CHUNK: usize = 65536;
/// The stated cap on how much `waitFor` will buffer for one stream. A child that
/// writes more than this before exiting cannot be waited for silently: `waitFor`
/// raises `ErrResourceBusy` rather than growing without bound (or, worse,
/// deadlocking again). Everything drained so far stays readable.
pub(crate) const SPILL_MAX_CAPACITY: usize = 16 * 1024 * 1024;

// The doubling growth walks powers of two from the initial capacity, so the cap
// must sit on that ladder for `newcap` to land on it exactly.
const _: () = assert!(SPILL_INITIAL_CAPACITY.is_power_of_two());
const _: () = assert!(SPILL_MAX_CAPACITY.is_power_of_two());
const _: () = assert!(SPILL_CHUNK <= SPILL_INITIAL_CAPACITY);

/// Branch to `ready` when `buf` (a spill block pointer, possibly null) still
/// holds bytes no reader has taken; otherwise fall through. `t0`/`t1` are
/// scratch. Used by `poll`, which must report a stream readable while the drained
/// bytes are in hand even though the fd itself may already be at EOF.
pub(crate) fn emit_spill_pending(
    symbol: &str,
    tag: &str,
    buf: impl Into<Operand> + Clone,
    t0: impl Into<Operand> + Clone,
    t1: impl Into<Operand> + Clone,
    ready: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let empty = format!("{symbol}_{tag}_spill_empty");
    instructions.extend([
        abi::compare_immediate(buf.clone(), "0"),
        abi::branch_eq(&empty),
        abi::load_u64(t0.clone(), buf.clone(), SPILL_OFFSET),
        abi::load_u64(t1.clone(), buf, SPILL_LENGTH),
        abi::compare_registers(t0, t1),
        abi::branch_lt(ready),
        abi::label(&empty),
    ]);
}

/// Take one byte out of `buf` into `dst` and advance the block's read offset, or
/// branch to `miss` when the block is absent or exhausted (the caller then reads
/// the fd). `t0`/`t1` are scratch.
pub(crate) fn emit_spill_take_byte(
    buf: impl Into<Operand> + Clone,
    dst: impl Into<Operand> + Clone,
    t0: impl Into<Operand> + Clone,
    t1: impl Into<Operand> + Clone,
    miss: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::compare_immediate(buf.clone(), "0"),
        abi::branch_eq(miss),
        abi::load_u64(t0.clone(), buf.clone(), SPILL_OFFSET),
        abi::load_u64(t1.clone(), buf.clone(), SPILL_LENGTH),
        abi::compare_registers(t0.clone(), t1.clone()),
        abi::branch_ge(miss),
        abi::add_immediate(t1.clone(), buf.clone(), SPILL_DATA),
        abi::add_registers(t1.clone(), t1.clone(), t0.clone()),
        abi::load_u8(dst, t1, 0),
        abi::add_immediate(t0.clone(), t0.clone(), 1),
        abi::store_u64(t0, buf, SPILL_OFFSET),
    ]);
}

/// Copy up to `max` buffered bytes out of `buf` into the memory at `dst`,
/// leaving the number copied in `count` and advancing the block's read offset.
/// Branches to `miss` when the block is absent or exhausted. `dst` is not
/// modified; `t0`..`t3` are scratch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_spill_take_chunk(
    symbol: &str,
    tag: &str,
    buf: impl Into<Operand> + Clone,
    dst: impl Into<Operand> + Clone,
    max: usize,
    count: impl Into<Operand> + Clone,
    t0: impl Into<Operand> + Clone,
    t1: impl Into<Operand> + Clone,
    t2: impl Into<Operand> + Clone,
    t3: impl Into<Operand> + Clone,
    miss: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let clamped = format!("{symbol}_{tag}_spill_clamped");
    let copy_loop = format!("{symbol}_{tag}_spill_copy");
    let copy_done = format!("{symbol}_{tag}_spill_copied");
    instructions.extend([
        abi::compare_immediate(buf.clone(), "0"),
        abi::branch_eq(miss),
        abi::load_u64(t0.clone(), buf.clone(), SPILL_OFFSET),
        abi::load_u64(t1.clone(), buf.clone(), SPILL_LENGTH),
        abi::compare_registers(t0.clone(), t1.clone()),
        abi::branch_ge(miss),
        // count = min(length - offset, max)
        abi::subtract_registers(count.clone(), t1.clone(), t0.clone()),
        abi::move_immediate(t1.clone(), "Integer", &max.to_string()),
        abi::compare_registers(count.clone(), t1.clone()),
        abi::branch_le(&clamped),
        abi::move_register(count.clone(), t1.clone()),
        abi::label(&clamped),
        // src = data + offset; consume the run up front.
        abi::add_immediate(t1.clone(), buf.clone(), SPILL_DATA),
        abi::add_registers(t1.clone(), t1.clone(), t0.clone()),
        abi::add_registers(t0.clone(), t0.clone(), count.clone()),
        abi::store_u64(t0.clone(), buf, SPILL_OFFSET),
        abi::move_register(t2.clone(), dst),
        abi::move_immediate(t0.clone(), "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(t0.clone(), count),
        abi::branch_ge(&copy_done),
        abi::load_u8(t3.clone(), t1.clone(), 0),
        abi::store_u8(t3, t2.clone(), 0),
        abi::add_immediate(t1.clone(), t1, 1),
        abi::add_immediate(t2.clone(), t2, 1),
        abi::add_immediate(t0.clone(), t0, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
    ]);
}

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
