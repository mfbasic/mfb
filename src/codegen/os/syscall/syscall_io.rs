//! Shared syscall / `EINTR`-retry emission primitives (bug-62).
//!
//! These are NOT `fs`-specific: the `EINTR`-retry guards, the raw-syscall-vs-libc
//! errno-convention split, and the short-transfer loop tail are used by every
//! fs/io/net/term read/write site (`io_stdout`/`io_stdin`/`net/`/`term_grid`/…),
//! so they live in the shared code layer rather than in the migrated `fs`
//! package's `native/` dir (the plan-72 transitivity rule: a helper called by a
//! non-`fs` function stays in `src/target`). Relocated verbatim from the former
//! `src/target/shared/code/fs/io.rs`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::target::shared::abi;
use std::collections::HashMap;
/// `EINTR` — a syscall interrupted by a signal handler before it transferred any
/// bytes. Its numeric value is `4` on both Linux and macOS/BSD (bug-62), so the
/// EINTR-retry guards can compare against a single literal on every backend.
/// `pub(crate)` so sibling emit sites (e.g. the `term::` present-write loop's
/// bug-410 retry test) can assert against the canonical value rather than a
/// restated `"4"`.
pub(crate) const EINTR_ERRNO: &str = "4";

/// Whether this program links the platform's `errno` accessor (`___error` on
/// macOS, `__errno_location` on Linux). Both `fs::` (a `File` only comes from
/// `fs::openFile`, which pulls the accessor in) and the `io::` read helpers
/// (`readByte`/`readChar`/`readLine`/`input` — their `plan.rs` arms co-import the
/// accessor, bug-62) link it, so their read/write/seek loops always read `errno`
/// and retry `EINTR`. The only path that links no accessor is a program whose sole
/// syscall use is an `io::` output drain (`io.print`/`io.write`/`io.flush`, never a
/// read and never `fs`): there the libc-write negative return cannot be classified
/// and is a hard error — acceptable, since a drain-only `EINTR` is degenerate and
/// `linux-x86_64`'s raw-`svc` write still retries via its `-errno` return. Checking
/// the merged import table keeps that boundary honest: the libc `EINTR` retry is
/// emitted exactly when `errno` is actually readable at runtime.
pub(crate) fn errno_accessor_available(platform_imports: &HashMap<String, String>) -> bool {
    platform_imports.contains_key("___error") || platform_imports.contains_key("__errno_location")
}

/// Whether `platform`'s `write` (used by every fs/io output loop, including the
/// stdout/File drains) is issued as a bare kernel `syscall` rather than through
/// the libc wrapper. Only the `linux-x86_64` backend does this — its `emit_write`
/// is a raw `svc`, so a failing `write` returns the negative `-errno` directly in
/// the return register and does NOT set the libc `errno` cell. Every other
/// backend's `write` (and every backend's `read`/`lseek`) goes through libc: a
/// `-1` return with the real code behind the `errno` accessor. The EINTR guard has
/// to read the two conventions differently, so the write sites consult this.
pub(crate) fn write_uses_raw_syscall(platform: &dyn CodegenPlatform) -> bool {
    platform.target() == "linux-x86_64"
}

/// Emit the tail of a fs/io read/write site for the case where the syscall return
/// (`ret`) has already been compared against `0` and is known to be negative
/// here. On `EINTR` — a signal interrupted the call before any byte moved —
/// branch back to `retry_label` to re-issue the identical syscall (the
/// loop-carried cursor and remaining count are unchanged); on any other error
/// branch to `error_label`.
///
/// Two conventions (bug-62):
/// * `raw_return` (the `linux-x86_64` raw-`svc` `write`): the return value is
///   `-errno`, so `EINTR` is exactly `ret == -EINTR`, tested as `ret + EINTR == 0`
///   with no libc call — this even works in a pure-`io::` program that never links
///   the accessor.
/// * otherwise (every libc `read`/`write`/`lseek`): re-read `errno` through the
///   platform accessor (`___error` / `__errno_location`, left in `x9`). `fs::` and
///   the `io::` read helpers import the accessor, so they retry `EINTR`. Only an
///   output-drain-only program (`io.print`/`io.write`/`io.flush` with no read and
///   no `fs`) omits it; there the negative return cannot be classified, so it is a
///   hard error.
///
/// `emit_errno` issues a `bl` to the accessor, which the register allocator treats
/// like any other call (all caller-saved integer registers clobbered); the
/// `retry_label`/`error_label` targets reload every value they need from vregs or
/// stack slots, so nothing live is read out of a caller-saved register across the
/// call (see `.ai/compiler.md`, "Native Codegen Register Lifetimes"). `x9` is the
/// established errno scratch and is dead on the negative-return path.
// The `(ret.clone()).into()` at the `emit_errno` call is load-bearing for emission
// (see the note there); `clippy::useless_conversion` is a false positive.
#[allow(clippy::useless_conversion)]
pub(crate) fn emit_eintr_retry_or_error(
    ctx: &mut EmitCtx,
    ret: impl Into<Operand>,
    raw_return: bool,
    retry_label: &str,
    error_label: &str,
) -> Result<(), String> {
    let ret = ret.into();
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // `ret` (the syscall return) is dead once we branch to retry/error here, so
    // reuse it as the errno scratch instead of naming a physical register
    // (plan-34-C): the retry edge reloads its cursor/remaining from spill slots.
    let eintr = EINTR_ERRNO
        .parse::<usize>()
        .expect("EINTR_ERRNO is numeric");
    if raw_return {
        // Raw-`svc` return is `-errno`: EINTR iff `ret == -EINTR`, i.e.
        // `ret + EINTR == 0`.
        ctx.instructions.extend([
            abi::add_immediate(&ret, &ret, eintr),
            abi::compare_immediate(&ret, "0"),
            abi::branch_eq(retry_label),
            abi::branch(error_label),
        ]);
    } else if errno_accessor_available(platform_imports) {
        // `emit_errno` loads the current `errno` into `ret` (reused).
        platform.emit_errno(
            symbol,
            // The `.into()` is load-bearing for emission (removing it changes the native
            // code — `clippy::useless_conversion` is a false positive here); kept to
            // preserve byte-identity with the pre-migration `code/fs/io.rs`.
            (ret.clone()).into(),
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
        ctx.instructions.extend([
            abi::compare_immediate(&ret, EINTR_ERRNO),
            abi::branch_eq(retry_label),
            abi::branch(error_label),
        ]);
    } else {
        ctx.instructions.push(abi::branch(error_label));
    }
    Ok(())
}

/// Advance-and-retry tail for a write/read loop whose body re-issues the syscall
/// at `loop_label` from the loop-carried `cursor`/`remaining` vregs (bug-51's
/// short-transfer loop, extended for bug-62). `ret` holds the syscall return: a
/// positive count advances the cursor and re-loops; a `0` return moved nothing for
/// a nonzero request and is a hard error (never a spin); a negative return is
/// `EINTR`-retried at `loop_label` or errored via [`emit_eintr_retry_or_error`].
/// `raw_return` selects the errno convention (see [`write_uses_raw_syscall`]);
/// pass `false` for every `read` loop (reads always go through libc).
pub(crate) fn emit_transfer_loop_tail(
    ctx: &mut EmitCtx,
    ret: impl Into<Operand>,
    raw_return: bool,
    cursor: &str,
    remaining: &str,
    loop_label: &str,
    error_label: &str,
) -> Result<(), String> {
    let ret = ret.into();
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let advance = format!("{loop_label}_advance");
    ctx.instructions.extend([
        abi::compare_immediate(&ret, "0"),
        abi::branch_gt(&advance),
        abi::branch_eq(error_label),
    ]);
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        &ret,
        raw_return,
        loop_label,
        error_label,
    )?;
    ctx.instructions.extend([
        abi::label(&advance),
        abi::add_registers(cursor, cursor, &ret),
        abi::subtract_registers(remaining, remaining, &ret),
        abi::branch(loop_label),
    ]);
    Ok(())
}

/// Guard the negative return of a single (non-advancing) `read` whose result in
/// `x0` has just been compared against `0` by the caller. A non-negative return
/// branches to `resume_label`; a negative return is `EINTR`-retried at
/// `retry_label` — which re-runs the syscall's argument setup — or errored. Reads
/// always go through libc on every backend, so this uses the `errno`-accessor
/// convention.
///
/// The caller emits its own follow-on branch on the same `x0 vs 0` comparison
/// (e.g. `branch_eq <eof>`) right after this guard. RISC-V has no persistent
/// condition flags — the MIR fuser welds each compare to the single branch that
/// immediately follows it — so the caller's `cmp x0, 0` is consumed by the
/// `branch_ge` here and cannot also feed the caller's branch. This guard therefore
/// re-issues `cmp x0, 0` at `resume_label`; `x0` is untouched on the `>= 0` path
/// (the guard body is skipped), so the re-comparison is exact and the caller's
/// branch fuses with it on every backend.
pub(crate) fn emit_single_op_eintr_guard(
    ctx: &mut EmitCtx,
    retry_label: &str,
    resume_label: &str,
    error_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.push(abi::branch_ge(resume_label));
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        abi::return_register(),
        false,
        retry_label,
        error_label,
    )?;
    ctx.instructions.extend([
        abi::label(resume_label),
        abi::compare_immediate(abi::return_register(), "0"),
    ]);
    Ok(())
}
