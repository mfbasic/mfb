//! The monotonic nanosecond reading the mouse ring stamps each event with.
//!
//! plan-94-B Corrections B1: none of the three monotonic emitters already in the
//! tree could be called from here.
//!
//! - `perf::emit_read_monotonic_nanos` hard-codes Darwin's `CLOCK_MONOTONIC = 6`
//!   with no platform branch, so it reads the wrong clock on Linux.
//! - `net::emit_monotonic_nanos` uses `platform.clock_monotonic()`, which is
//!   `unreachable!()` on Win64 — calling it from a Windows build panics the
//!   *compiler*.
//! - `datetime::monotonicNanos` is correct on all three families but is a whole
//!   member body: it writes `RESULT_VALUE_REGISTER`, branches to a caller-supplied
//!   `ErrOverflow` label, and addresses datetime-local frame constants.
//!
//! So this is the fourth — but the first one shaped as a reusable emitter: it
//! takes its destination register and its scratch offset, and it is correct on
//! every family. The arithmetic is lifted from `datetime::monotonicNanos`, which
//! is the version that had already solved the Windows half.
//!
//! **It deliberately does not trap on overflow**, which is the one place it
//! diverges from datetime's. The stamp is consumed only as `now − stamp ≤ TTL`,
//! and wrapping unsigned subtraction is *correct* for any interval shorter than
//! 584 years. Trapping would add a failure mode to the stdin read path in exchange
//! for nothing — a narrower contract than `datetime::monotonicNanos`, deliberately,
//! which is why this is a separate emitter rather than a call into that member.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::target::shared::abi;

/// `CLOCK_MONOTONIC` on Darwin. Linux uses `1`; the two differ, which is exactly
/// what `perf`'s emitter gets wrong.
const CLOCK_MONOTONIC_DARWIN: &str = "6";
/// `CLOCK_MONOTONIC` on Linux.
const CLOCK_MONOTONIC_LINUX: &str = "1";
const NANOS_PER_SEC: &str = "1000000000";

/// Bytes of caller stack scratch this emitter addresses at `scratch`.
///
/// POSIX needs 16 (a `struct timespec`); Windows needs 16 (two `LARGE_INTEGER`s).
/// One number for both keeps every caller's frame arithmetic uniform.
pub(crate) const MOUSE_CLOCK_SCRATCH_BYTES: usize = 16;

/// Read the monotonic clock into `dst` as whole nanoseconds.
///
/// `scratch` is an sp-relative byte offset of [`MOUSE_CLOCK_SCRATCH_BYTES`] the
/// caller has reserved. `dst` must be a vreg (it is written after the external
/// call, so nothing is held across it).
pub(crate) fn emit_monotonic_nanos(
    dst: &str,
    scratch: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    if ctx.platform.family() == PlatformFamily::Windows {
        emit_windows_qpc_nanos(dst, scratch, ctx, vregs)
    } else {
        emit_posix_clock_nanos(dst, scratch, ctx, vregs)
    }
}

/// `clock_gettime(CLOCK_MONOTONIC, &ts)` → `ts.tv_sec * 1e9 + ts.tv_nsec`.
fn emit_posix_clock_nanos(
    dst: &str,
    scratch: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let clock_id = match ctx.platform.family() {
        PlatformFamily::MacOS => CLOCK_MONOTONIC_DARWIN,
        PlatformFamily::Linux => CLOCK_MONOTONIC_LINUX,
        PlatformFamily::Windows => unreachable!("routed to QueryPerformanceCounter above"),
    };
    ctx.instructions
        .push(abi::move_immediate(abi::c_arg(0), "Integer", clock_id));
    ctx.instructions.push(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        scratch,
    ));
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    platform.emit_external_call(
        "clock_gettime",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    let sec = vregs.next();
    let nsec = vregs.next();
    let scale = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(&sec, abi::stack_pointer(), scratch),
        abi::load_u64(&nsec, abi::stack_pointer(), scratch + 8),
        abi::move_immediate(&scale, "Integer", NANOS_PER_SEC),
        abi::multiply_registers(&sec, &sec, &scale),
        abi::add_registers(dst, &sec, &nsec),
    ]);
    Ok(())
}

/// `QueryPerformanceCounter`/`QueryPerformanceFrequency` → nanoseconds.
///
/// Split across the quotient and the remainder —
/// `(counter/freq)*1e9 + ((counter%freq)*1e9)/freq` — because `counter*1e9` alone
/// overflows `u64` within about 21 seconds at a 10 MHz frequency. Lifted from
/// `datetime::monotonicNanos`, minus its overflow branches (see the module doc).
fn emit_windows_qpc_nanos(
    dst: &str,
    scratch: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.push(abi::add_immediate(
        abi::c_arg(0),
        abi::stack_pointer(),
        scratch,
    ));
    platform.emit_external_call(
        "QueryPerformanceCounter",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.push(abi::add_immediate(
        abi::c_arg(0),
        abi::stack_pointer(),
        scratch + 8,
    ));
    platform.emit_external_call(
        "QueryPerformanceFrequency",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;

    let counter = vregs.next();
    let freq = vregs.next();
    let q = vregs.next();
    let rem = vregs.next();
    let scale = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(&counter, abi::stack_pointer(), scratch),
        abi::load_u64(&freq, abi::stack_pointer(), scratch + 8),
        abi::unsigned_divide_registers(&q, &counter, &freq),
        abi::multiply_subtract_registers(&rem, &q, &freq, &counter),
        abi::move_immediate(&scale, "Integer", NANOS_PER_SEC),
        abi::multiply_registers(&q, &q, &scale),
        abi::multiply_registers(&rem, &rem, &scale),
        abi::unsigned_divide_registers(&rem, &rem, &freq),
        abi::add_registers(dst, &q, &rem),
    ]);
    Ok(())
}
