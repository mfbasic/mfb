//! Native code generation for the `datetime::` OS-seam intrinsics
//! (plan-01-datetime.md §8.2). Three tiny runtime helpers wrap libc:
//!
//! - `datetime.nowNanos` — `clock_gettime(CLOCK_REALTIME)` → `sec*1e9 + nsec`.
//! - `datetime.monotonicNanos` — `clock_gettime(CLOCK_MONOTONIC)` → nanoseconds.
//! - `datetime.localOffset` — `localtime_r(&epochSeconds, &tm)` → `tm_gmtoff`.
//!
//! `nowNanos` / `monotonicNanos` always succeed and return an `Integer` in the
//! standard result-value register with the OK tag set. `localOffset` takes an
//! unvalidated user-supplied instant: `localtime_r` returns `NULL` (setting
//! `EOVERFLOW`) when the year does not fit `tm_year`'s `int`, leaving `tm`
//! untouched, so that arm branches on the return and raises `ErrInvalidArgument`
//! rather than reading an uninitialized stack qword (bug-42). The portable
//! calendar math that consumes these lives in `datetime_package.mfb`.

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

// Frame layout (16-aligned). `LOCALS_SIZE` is the size of this locals region,
// which `finalize_vreg_body_with_locals` rounds to 16 and reserves; the vreg
// frame owns saving the link register, not a slot named here.
const TIMESPEC_OFFSET: usize = 0; // struct timespec { tv_sec; tv_nsec } (16 bytes)
const TIME_T_OFFSET: usize = 0; // time_t input to localtime_r (reuses the low slot)
const TM_OFFSET: usize = 16; // struct tm output (>= 56 bytes)
const LOCALS_SIZE: usize = 88;

// `CLOCK_REALTIME` is 0 on both Linux and macOS. `CLOCK_MONOTONIC` diverges:
// Linux uses 1, macOS (Darwin) uses 6.
const CLOCK_REALTIME: &str = "0";
const CLOCK_MONOTONIC_LINUX: &str = "1";
const CLOCK_MONOTONIC_DARWIN: &str = "6";

// `struct tm.tm_gmtoff` (a `long`) follows the nine leading `int` fields
// (`9 * 4 = 36`, padded to 8-byte alignment) on both glibc and Darwin BSD libc.
const TM_GMTOFF_OFFSET: usize = 40;

// --- Windows (plan-66-A) ------------------------------------------------------
//
// Windows has no libc `clock_gettime`/`localtime_r`, so its three datetime
// intrinsics ride Win32 kernel32 calls instead (import rows in
// `win_x86_64/plan.rs`). The buffers below are laid out sp-relative in the same
// `LOCALS_SIZE` frame the libc path reserves; the two paths never both execute.
//
//  - `monotonicNanos`: QueryPerformanceCounter/QueryPerformanceFrequency, then
//    an overflow-safe tick→nanosecond conversion (a naive `ticks * 1e9` overflows
//    u64 within ~21 s at the usual 10 MHz frequency).
//  - `nowNanos`: GetSystemTimePreciseAsFileTime → 100 ns intervals since 1601;
//    rebased to Unix nanoseconds.
//  - `localOffset`: FileTimeToSystemTime → SystemTimeToTzSpecificLocalTime →
//    SystemTimeToFileTime; the local/UTC FILETIME delta IS the offset. A NULL
//    return (year out of FILETIME/SYSTEMTIME range) raises `ErrInvalidArgument`
//    exactly as the libc `localtime_r` NULL path does (bug-42).
const WIN_FILETIME_OFFSET: usize = 0; // FILETIME (u64, 100 ns since 1601)
const WIN_QPC_FREQ_OFFSET: usize = 8; // QueryPerformanceFrequency out (u64)
const WIN_UTC_SYSTEMTIME_OFFSET: usize = 16; // SYSTEMTIME (16 bytes)
const WIN_LOCAL_SYSTEMTIME_OFFSET: usize = 32; // SYSTEMTIME (16 bytes)
const WIN_LOCAL_FILETIME_OFFSET: usize = 48; // FILETIME (u64)

// 100 ns intervals between 1601-01-01 and 1970-01-01 (134774 days × 86400 s ×
// 1e7). Rebases a Windows FILETIME onto the Unix epoch.
const WIN_FILETIME_UNIX_EPOCH_100NS: &str = "116444736000000000";
// 100 ns intervals per second.
const WIN_HUNDRED_NS_PER_SEC: &str = "10000000";
const NANOS_PER_SEC: &str = "1000000000";
// Seconds between 1970-01-01 and 1601-01-01 (the FILETIME epoch), positive.
// `epochSeconds + this < 0` ⟺ the instant predates 1601 (a negative FILETIME).
const WIN_UNIX_EPOCH_TO_1601_SEC: &str = "11644473600";
// Largest Unix-epoch second whose FILETIME (`*1e7 + WIN_FILETIME_UNIX_EPOCH_100NS`)
// still fits i64; a larger value would wrap. `(i64::MAX - epoch) / 1e7`.
const WIN_FILETIME_MAX_UNIX_SEC: &str = "910692730085";

pub(super) fn lower_datetime_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2): the timespec/tm buffer is an explicit
    // sp-relative local region; the x9-x11 scratch becomes vregs.
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    // Declared up front so the `localOffset` arm can branch to it and the shared
    // error tail below can define it: the out-of-range failure label for
    // `localtime_r` returning NULL (bug-42).
    let localoffset_range_fail = format!("{symbol}_range");

    if platform.family() == PlatformFamily::Windows {
        // Windows has no libc clocks; route to the kernel32 lowering (plan-66-A).
        // The shared OK tail and the `localOffset` range-fail tail below are reused
        // unchanged — the Windows body sets `RESULT_VALUE_REGISTER` and branches to
        // `localoffset_range_fail` on error exactly like the libc path.
        lower_datetime_windows(
            call,
            symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &localoffset_range_fail,
        )?;
    } else {
        match call {
            "datetime.nowNanos" | "datetime.monotonicNanos" => {
                let clock_id = if call == "datetime.nowNanos" {
                    CLOCK_REALTIME
                } else {
                    match platform.family() {
                        PlatformFamily::MacOS => CLOCK_MONOTONIC_DARWIN,
                        PlatformFamily::Linux => CLOCK_MONOTONIC_LINUX,
                        // Windows is routed to `lower_datetime_windows` above and never
                        // reaches this libc clock-id selection (plan-66-A).
                        PlatformFamily::Windows => {
                            unreachable!("plan-66-A routes Windows datetime to kernel32")
                        }
                    }
                };
                // x0 = clock id, x1 = &timespec.
                instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", clock_id));
                instructions.push(abi::add_immediate(
                    abi::c_arg(1),
                    abi::stack_pointer(),
                    TIMESPEC_OFFSET,
                ));
                platform.emit_libc_call(
                    "clock_gettime",
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
                // nanos = tv_sec * 1_000_000_000 + tv_nsec.
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), TIMESPEC_OFFSET),
                    abi::load_u64("%v10", abi::stack_pointer(), TIMESPEC_OFFSET + 8),
                    abi::move_immediate("%v11", "Integer", "1000000000"),
                    abi::multiply_registers("%v9", "%v9", "%v11"),
                    abi::add_registers(RESULT_VALUE_REGISTER, "%v9", "%v10"),
                ]);
            }
            "datetime.localOffset" => {
                // x0 holds epochSeconds. Stash it as the `time_t` input, then call
                // `localtime_r(&time_t, &tm)` and read `tm.tm_gmtoff`.
                instructions.extend([
                    abi::store_u64(abi::c_arg(0), abi::stack_pointer(), TIME_T_OFFSET),
                    abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), TIME_T_OFFSET),
                    abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), TM_OFFSET),
                ]);
                platform.emit_libc_call(
                    "localtime_r",
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
                // `localtime_r` returns NULL (and sets EOVERFLOW) when the instant's
                // year does not fit `tm_year`'s `int`; on that path it writes no field
                // of `tm`, so loading `tm_gmtoff` would return an uninitialized stack
                // qword (an ASLR info-leak). Branch on the return before touching the
                // buffer (bug-42). plan-85: `localtime_r`'s pointer return is a C
                // result (`rax`, `%retC`), not the aligned MFB result register — read
                // it from the C-return register (byte-identical `x0` on ARM/RISC-V).
                instructions.push(abi::compare_immediate(abi::c_return(0), "0"));
                instructions.push(abi::branch_eq(&localoffset_range_fail));
                instructions.push(abi::load_u64(
                    RESULT_VALUE_REGISTER,
                    abi::stack_pointer(),
                    TM_OFFSET + TM_GMTOFF_OFFSET,
                ));
            }
            other => {
                return Err(format!(
                    "native datetime lowering does not support runtime call '{other}'"
                ));
            }
        }
    }

    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());

    if call == "datetime.localOffset" {
        // Out-of-range `epochSeconds`: report `ErrInvalidArgument` rather than
        // returning `tm.tm_gmtoff` from a buffer `localtime_r` never wrote. The
        // runtime-helper call site (`emit_runtime_helper_call`) already checks the
        // tag and auto-propagates the error up through `offsetAt`/`toLocal`, so no
        // package-source change is needed (bug-42). This tail sits after the shared
        // OK return so success never falls into it.
        instructions.push(abi::label(&localoffset_range_fail));
        instructions.push(abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            ERR_INVALID_ARGUMENT_CODE,
        ));
        instructions.push(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        push_error_message_address(
            symbol,
            ERR_INVALID_ARGUMENT_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::return_());
    }

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCALS_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

/// The Windows kernel32 body for the three `datetime::` intrinsics (plan-66-A).
/// Emits into `instructions`/`relocations`; the caller supplies the shared OK tail
/// and the `localOffset` range-fail tail (`range_fail`). Every OS call rides
/// `platform.emit_libc_call`, whose import library comes from the plan's
/// `runtime_imports` map (kernel32.dll for all of these).
#[allow(clippy::too_many_arguments)]
fn lower_datetime_windows(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    range_fail: &str,
) -> Result<(), String> {
    let call_win = |func: &str,
                    instructions: &mut Vec<CodeInstruction>,
                    relocations: &mut Vec<CodeRelocation>|
     -> Result<(), String> {
        platform.emit_libc_call(func, symbol, platform_imports, instructions, relocations)
    };

    match call {
        "datetime.monotonicNanos" => {
            // QueryPerformanceCounter(&counter); QueryPerformanceFrequency(&freq).
            instructions.push(abi::add_immediate(
                abi::c_arg(0),
                abi::stack_pointer(),
                WIN_FILETIME_OFFSET,
            ));
            call_win("QueryPerformanceCounter", instructions, relocations)?;
            instructions.push(abi::add_immediate(
                abi::c_arg(0),
                abi::stack_pointer(),
                WIN_QPC_FREQ_OFFSET,
            ));
            call_win("QueryPerformanceFrequency", instructions, relocations)?;
            // nanos = (counter/freq)*1e9 + ((counter%freq)*1e9)/freq. Splitting the
            // multiply across the quotient and remainder keeps every intermediate
            // inside u64: `counter*1e9` alone overflows within ~21 s at 10 MHz.
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), WIN_FILETIME_OFFSET), // counter
                abi::load_u64("%v10", abi::stack_pointer(), WIN_QPC_FREQ_OFFSET), // freq
                abi::unsigned_divide_registers("%v11", "%v9", "%v10"),           // q
                abi::multiply_subtract_registers("%v12", "%v11", "%v10", "%v9"), // rem = counter - q*freq
                abi::move_immediate("%v13", "Integer", NANOS_PER_SEC),
                abi::multiply_registers("%v11", "%v11", "%v13"), // q*1e9
                abi::multiply_registers("%v12", "%v12", "%v13"), // rem*1e9
                abi::unsigned_divide_registers("%v12", "%v12", "%v10"), // (rem*1e9)/freq
                abi::add_registers(RESULT_VALUE_REGISTER, "%v11", "%v12"),
            ]);
        }
        "datetime.nowNanos" => {
            // GetSystemTimePreciseAsFileTime(&ft): 100 ns intervals since 1601.
            instructions.push(abi::add_immediate(
                abi::c_arg(0),
                abi::stack_pointer(),
                WIN_FILETIME_OFFSET,
            ));
            call_win("GetSystemTimePreciseAsFileTime", instructions, relocations)?;
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), WIN_FILETIME_OFFSET),
                abi::move_immediate("%v10", "Integer", WIN_FILETIME_UNIX_EPOCH_100NS),
                abi::subtract_registers("%v9", "%v9", "%v10"), // 100 ns since Unix epoch
                abi::move_immediate("%v10", "Integer", "100"),
                abi::multiply_registers(RESULT_VALUE_REGISTER, "%v9", "%v10"),
            ]);
        }
        "datetime.localOffset" => {
            // epochSeconds (ARG[0]) → FILETIME → UTC SYSTEMTIME → local SYSTEMTIME
            // → local FILETIME. The (local − original) FILETIME delta is the UTC
            // offset in 100 ns units; a NULL from any conversion means the instant
            // is out of the FILETIME/SYSTEMTIME range → ErrInvalidArgument (bug-42).
            //
            // First bound `epochSeconds` so the `*1e7 + epoch` FILETIME arithmetic
            // cannot wrap: a wrapped product yields a valid-looking FILETIME that
            // FileTimeToSystemTime accepts, silently returning a garbage offset
            // instead of trapping (the libc `localtime_r` NULL path traps because
            // `tm_year`'s `int` overflows). The bounds are the exact FILETIME range:
            // below -11644473600 s the FILETIME is negative; above the HIGH bound
            // `epochSeconds*1e7 + epoch` exceeds i64. The residual year>30827 edge is
            // still caught by the FileTimeToSystemTime NULL check downstream.
            instructions.extend([
                abi::move_register("%v9", abi::c_arg(0)), // epochSeconds
                // HIGH: epochSeconds > 910692730085 → epochSeconds*1e7+epoch > i64max.
                abi::move_immediate("%v10", "Integer", WIN_FILETIME_MAX_UNIX_SEC),
                abi::compare_registers("%v9", "%v10"),
                abi::branch_gt(range_fail),
                // LOW: epochSeconds + 11644473600 < 0 → FILETIME negative (pre-1601).
                abi::move_immediate("%v10", "Integer", WIN_UNIX_EPOCH_TO_1601_SEC),
                abi::add_registers("%v10", "%v9", "%v10"),
                abi::compare_immediate("%v10", "0"),
                abi::branch_lt(range_fail),
                abi::move_immediate("%v10", "Integer", WIN_HUNDRED_NS_PER_SEC),
                abi::multiply_registers("%v9", "%v9", "%v10"), // epochSeconds*1e7
                abi::move_immediate("%v10", "Integer", WIN_FILETIME_UNIX_EPOCH_100NS),
                abi::add_registers("%v9", "%v9", "%v10"), // FILETIME
                abi::store_u64("%v9", abi::stack_pointer(), WIN_FILETIME_OFFSET),
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), WIN_FILETIME_OFFSET),
                abi::add_immediate(
                    abi::c_arg(1),
                    abi::stack_pointer(),
                    WIN_UTC_SYSTEMTIME_OFFSET,
                ),
            ]);
            call_win("FileTimeToSystemTime", instructions, relocations)?;
            instructions.push(abi::compare_immediate(abi::RET[0], "0"));
            instructions.push(abi::branch_eq(range_fail));
            // SystemTimeToTzSpecificLocalTime(NULL, &utc, &local): NULL selects the
            // machine's current time zone, applying its DST rules to the instant.
            instructions.extend([
                abi::move_immediate(abi::c_arg(0), "Integer", "0"),
                abi::add_immediate(
                    abi::c_arg(1),
                    abi::stack_pointer(),
                    WIN_UTC_SYSTEMTIME_OFFSET,
                ),
                abi::add_immediate(
                    abi::c_arg(2),
                    abi::stack_pointer(),
                    WIN_LOCAL_SYSTEMTIME_OFFSET,
                ),
            ]);
            call_win("SystemTimeToTzSpecificLocalTime", instructions, relocations)?;
            instructions.push(abi::compare_immediate(abi::RET[0], "0"));
            instructions.push(abi::branch_eq(range_fail));
            // SystemTimeToFileTime(&local, &localFt).
            instructions.extend([
                abi::add_immediate(
                    abi::c_arg(0),
                    abi::stack_pointer(),
                    WIN_LOCAL_SYSTEMTIME_OFFSET,
                ),
                abi::add_immediate(
                    abi::c_arg(1),
                    abi::stack_pointer(),
                    WIN_LOCAL_FILETIME_OFFSET,
                ),
            ]);
            call_win("SystemTimeToFileTime", instructions, relocations)?;
            instructions.push(abi::compare_immediate(abi::RET[0], "0"));
            instructions.push(abi::branch_eq(range_fail));
            // offsetSeconds = (localFt − ft) / 1e7, signed (west-of-UTC is negative).
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), WIN_LOCAL_FILETIME_OFFSET),
                abi::load_u64("%v10", abi::stack_pointer(), WIN_FILETIME_OFFSET),
                abi::subtract_registers("%v9", "%v9", "%v10"),
                abi::move_immediate("%v10", "Integer", WIN_HUNDRED_NS_PER_SEC),
                abi::signed_divide_registers(RESULT_VALUE_REGISTER, "%v9", "%v10"),
            ]);
        }
        other => {
            return Err(format!(
                "native Windows datetime lowering does not support runtime call '{other}'"
            ));
        }
    }
    Ok(())
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
