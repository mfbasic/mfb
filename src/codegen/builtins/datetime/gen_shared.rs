//! Shared code generation for the `datetime::` OS-seam intrinsics
//! (plan-01-datetime.md §8.2). Three tiny runtime helpers wrap libc; each owns its
//! `abi_function` body in its own `func_*.rs` (`lower_now_nanos`,
//! `lower_monotonic_nanos`, `lower_local_offset`), and the wrapper finalizes it
//! (crypto/io's clean-room shape):
//!
//! - `datetime.nowNanos` — `clock_gettime(CLOCK_REALTIME)` → `sec*1e9 + nsec`.
//! - `datetime.monotonicNanos` — `clock_gettime(CLOCK_MONOTONIC)` → nanoseconds.
//! - `datetime.localOffset` — `localtime_r(&epochSeconds, &tm)` → `tm_gmtoff`.
//!
//! `nowNanos` / `monotonicNanos` return an `Integer` in the standard result-value
//! register with the OK tag set, or raise `ErrOverflow` when the reading's
//! nanosecond count does not fit an `Integer` (bug-640: the fold used to wrap).
//! Their libc path — the `clock_gettime` call plus the checked `sec*1e9 + nsec`
//! fold — is the one genuinely shared emitter and lives here as
//! [`emit_libc_clock_nanos`], with the fail tail as [`emit_clock_overflow_tail`];
//! the two members differ only in the clock id they resolve and pass in. `localOffset` takes an
//! unvalidated user-supplied instant: `localtime_r` returns `NULL` (setting
//! `EOVERFLOW`) when the year does not fit `tm_year`'s `int`, leaving `tm`
//! untouched, so that member branches on the return and raises `ErrInvalidArgument`
//! rather than reading an uninitialized stack qword (bug-42). The portable
//! calendar math that consumes these lives in the injected `__datetime_*` bodies.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::raise_error_into;

use crate::target::shared::abi;
use crate::types::ParameterType;
// Frame layout (16-aligned). `LOCALS_SIZE` is the size of this locals region,
// which `finalize_vreg_body_with_locals` rounds to 16 and reserves; the vreg
// frame owns saving the link register, not a slot named here.
pub(crate) const TIMESPEC_OFFSET: usize = 0; // struct timespec { tv_sec; tv_nsec } (16 bytes)
pub(crate) const TIME_T_OFFSET: usize = 0; // time_t input to localtime_r (reuses the low slot)
pub(crate) const TM_OFFSET: usize = 16; // struct tm output (>= 56 bytes)
pub(crate) const LOCALS_SIZE: usize = 88;

// `CLOCK_REALTIME` is 0 on both Linux and macOS. `CLOCK_MONOTONIC` diverges:
// Linux uses 1, macOS (Darwin) uses 6.
pub(crate) const CLOCK_REALTIME: &str = "0";
pub(crate) const CLOCK_MONOTONIC_LINUX: &str = "1";
pub(crate) const CLOCK_MONOTONIC_DARWIN: &str = "6";

// `struct tm.tm_gmtoff` (a `long`) follows the nine leading `int` fields
// (`9 * 4 = 36`, padded to 8-byte alignment) on both glibc and Darwin BSD libc.
pub(crate) const TM_GMTOFF_OFFSET: usize = 40;

// --- Windows (plan-66-A) ------------------------------------------------------
//
// Windows has no libc `clock_gettime`/`localtime_r`, so its three datetime
// intrinsics ride Win32 kernel32 calls instead (import rows in
// `win_x86_64/plan.rs`). The buffers below are laid out sp-relative in the same
// `LOCALS_SIZE` frame the libc path reserves; the two paths never both execute.
//
//  - `monotonicNanos`: QueryPerformanceCounter/QueryPerformanceFrequency, then
//    a quotient/remainder tick→nanosecond conversion (a naive `ticks * 1e9`
//    overflows u64 within ~21 s at the usual 10 MHz frequency), with the
//    `quotient * 1e9` scale and the final add checked (bug-640).
//  - `nowNanos`: GetSystemTimePreciseAsFileTime → 100 ns intervals since 1601;
//    rebased to Unix nanoseconds with a checked `* 100` (bug-640).
//  - `localOffset`: FileTimeToSystemTime → SystemTimeToTzSpecificLocalTime →
//    SystemTimeToFileTime; the local/UTC FILETIME delta IS the offset. A NULL
//    return (year out of FILETIME/SYSTEMTIME range) raises `ErrInvalidArgument`
//    exactly as the libc `localtime_r` NULL path does (bug-42).
pub(crate) const WIN_FILETIME_OFFSET: usize = 0; // FILETIME (u64, 100 ns since 1601)
pub(crate) const WIN_QPC_FREQ_OFFSET: usize = 8; // QueryPerformanceFrequency out (u64)
pub(crate) const WIN_UTC_SYSTEMTIME_OFFSET: usize = 16; // SYSTEMTIME (16 bytes)
pub(crate) const WIN_LOCAL_SYSTEMTIME_OFFSET: usize = 32; // SYSTEMTIME (16 bytes)
pub(crate) const WIN_LOCAL_FILETIME_OFFSET: usize = 48; // FILETIME (u64)

// 100 ns intervals between 1601-01-01 and 1970-01-01 (134774 days × 86400 s ×
// 1e7). Rebases a Windows FILETIME onto the Unix epoch.
pub(crate) const WIN_FILETIME_UNIX_EPOCH_100NS: &str = "116444736000000000";
// 100 ns intervals per second.
pub(crate) const WIN_HUNDRED_NS_PER_SEC: &str = "10000000";
pub(crate) const NANOS_PER_SEC: &str = "1000000000";
// Seconds between 1970-01-01 and 1601-01-01 (the FILETIME epoch), positive.
// `epochSeconds + this < 0` ⟺ the instant predates 1601 (a negative FILETIME).
pub(crate) const WIN_UNIX_EPOCH_TO_1601_SEC: &str = "11644473600";
// Largest Unix-epoch second whose FILETIME (`*1e7 + WIN_FILETIME_UNIX_EPOCH_100NS`)
// still fits i64; a larger value would wrap. `(i64::MAX - epoch) / 1e7`.
pub(crate) const WIN_FILETIME_MAX_UNIX_SEC: &str = "910692730085";

/// The shared libc clock reading for `nowNanos` / `monotonicNanos`:
/// `clock_gettime(clock_id, &timespec)` then `nanos = tv_sec*1e9 + tv_nsec` into
/// the result-value register, branching to `overflow` when that count does not
/// fit an `Integer` (bug-640). The two members differ only in the `clock_id` they
/// resolve and pass in; the vreg allocation order (`sec`, `nsec`, `scale`, `high`,
/// `sign`, `carry`) and the timespec frame slot are identical, so both bodies
/// stay byte-identical.
///
/// The count is folded as a 128-bit value and range-checked once at the end,
/// rather than checking the multiply and the add separately: a separate multiply
/// check would reject `(-9223372037, 145224192)`, whose product leaves the
/// `Integer` range but whose sum is exactly `Integer` min. The high word of
/// `tv_sec * 1e9` is the signed-high multiply; `tv_nsec` is added with its sign
/// extension as its high word; the sum fits exactly when its high word equals
/// the sign extension of its low word. The caller owns the `overflow` label and
/// places [`emit_clock_overflow_tail`] at it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_libc_clock_nanos(
    clock_id: &str,
    symbol: &str,
    overflow: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    // x0 = clock id, x1 = &timespec.
    instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", clock_id));
    instructions.push(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        TIMESPEC_OFFSET,
    ));
    platform.emit_external_call(
        "clock_gettime",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    // nanos = tv_sec * 1_000_000_000 + tv_nsec, in 128 bits (`high:sec`). Every
    // value lives in a vreg allocated after the call, so nothing is held across it.
    let sec = vregs.next();
    let nsec = vregs.next();
    let scale = vregs.next();
    let high = vregs.next();
    let sign = vregs.next();
    let carry = vregs.next();
    instructions.extend([
        abi::load_u64(&sec, abi::stack_pointer(), TIMESPEC_OFFSET),
        abi::load_u64(&nsec, abi::stack_pointer(), TIMESPEC_OFFSET + 8),
        abi::move_immediate(&scale, "Integer", NANOS_PER_SEC),
        // The signed high word reads `sec` before the low multiply overwrites it.
        abi::signed_multiply_high_registers(&high, &sec, &scale),
        abi::multiply_registers(&sec, &sec, &scale),
        // Add `tv_nsec` sign-extended: low words with carry-out, then high words.
        abi::arithmetic_shift_right_immediate(&sign, &nsec, 63),
        abi::add_carry(&sec, &carry, &sec, &nsec, abi::ZERO),
        abi::add_carry(&high, abi::ZERO, &high, &sign, &carry),
        // Fits an `Integer` exactly when the high word is the low word's sign.
        abi::arithmetic_shift_right_immediate(&sign, &sec, 63),
        abi::compare_registers(&high, &sign),
        abi::branch_ne(overflow),
        abi::move_register(RESULT_VALUE_REGISTER, &sec),
    ]);
    Ok(())
}

/// The fail tail of `nowNanos` / `monotonicNanos` (bug-640): at `overflow`, raise
/// `ErrOverflow` and return. The runtime-helper call site checks the tag, stamps
/// the call-site origin, and propagates, as it does for `localOffset`'s
/// `ErrInvalidArgument`. Emit it after the OK return so success never falls into it.
pub(crate) fn emit_clock_overflow_tail(
    symbol: &str,
    overflow: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::label(overflow));
    raise_error_into(symbol, "ErrOverflow", instructions, relocations);
    instructions.push(abi::return_());
}

/// The `void` result every datetime OS-seam member returns: the body emitted its
/// own fallible ABI (the OK tail, plus the overflow tail for `nowNanos` /
/// `monotonicNanos` and the range-fail tail for `localOffset`), so
/// the `abi_function` wrapper appends no epilogue. `type_` is `Integer`.
pub(crate) fn void_int_result(call: &str) -> ValueResult {
    ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from("void"),
        text: call.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // bug-411: `WIN_FILETIME_MAX_UNIX_SEC` is the HIGH bound the Windows
    // `localOffset`/`offsetAt`/`toLocal` guard uses to reject an `epochSeconds`
    // whose FILETIME (`epochSeconds*1e7 + epoch`) would exceed i64 and wrap. Its
    // own doc-comment defines it as `(i64::MAX - epoch) / 1e7`; a constant even
    // one larger admits a value whose FILETIME exceeds i64, re-opening the exact
    // wrap the guard closes. Pin the constant to that formula so a stray digit
    // (the original `...477` typo, ~1000× too large) can never return.
    #[test]
    fn win_filetime_max_unix_sec_matches_no_wrap_formula() {
        let epoch: i64 = WIN_FILETIME_UNIX_EPOCH_100NS.parse().unwrap();
        let hundred_ns_per_sec: i64 = WIN_HUNDRED_NS_PER_SEC.parse().unwrap();
        let bound: i64 = WIN_FILETIME_MAX_UNIX_SEC.parse().unwrap();

        // The documented formula: the largest Unix second whose FILETIME fits i64.
        let expected = (i64::MAX - epoch) / hundred_ns_per_sec;
        assert_eq!(
            bound, expected,
            "WIN_FILETIME_MAX_UNIX_SEC must equal (i64::MAX - epoch)/1e7"
        );

        // Boundary invariant: `bound` itself does not wrap, `bound + 1` does.
        assert!(
            bound
                .checked_mul(hundred_ns_per_sec)
                .and_then(|v| v.checked_add(epoch))
                .is_some(),
            "bound*1e7 + epoch must still fit i64"
        );
        assert!(
            (bound + 1)
                .checked_mul(hundred_ns_per_sec)
                .and_then(|v| v.checked_add(epoch))
                .is_none(),
            "(bound+1)*1e7 + epoch must overflow i64 — bound is the exact max"
        );
    }
}
