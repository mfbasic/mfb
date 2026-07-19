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

// Frame layout (16-aligned). The saved link register sits at the top, clear of
// the libc scratch buffers below it.
const TIMESPEC_OFFSET: usize = 0; // struct timespec { tv_sec; tv_nsec } (16 bytes)
const TIME_T_OFFSET: usize = 0; // time_t input to localtime_r (reuses the low slot)
const TM_OFFSET: usize = 16; // struct tm output (>= 56 bytes)
const LR_OFFSET: usize = 88;

// `CLOCK_REALTIME` is 0 on both Linux and macOS. `CLOCK_MONOTONIC` diverges:
// Linux uses 1, macOS (Darwin) uses 6.
const CLOCK_REALTIME: &str = "0";
const CLOCK_MONOTONIC_LINUX: &str = "1";
const CLOCK_MONOTONIC_DARWIN: &str = "6";

// `struct tm.tm_gmtoff` (a `long`) follows the nine leading `int` fields
// (`9 * 4 = 36`, padded to 8-byte alignment) on both glibc and Darwin BSD libc.
const TM_GMTOFF_OFFSET: usize = 40;

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

    match call {
        "datetime.nowNanos" | "datetime.monotonicNanos" => {
            let clock_id = if call == "datetime.nowNanos" {
                CLOCK_REALTIME
            } else if platform.target().starts_with("macos") {
                CLOCK_MONOTONIC_DARWIN
            } else {
                CLOCK_MONOTONIC_LINUX
            };
            // x0 = clock id, x1 = &timespec.
            instructions.push(abi::move_immediate(abi::ARG[0], "Integer", clock_id));
            instructions.push(abi::add_immediate(
                abi::ARG[1],
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
                abi::store_u64(abi::ARG[0], abi::stack_pointer(), TIME_T_OFFSET),
                abi::add_immediate(abi::ARG[0], abi::stack_pointer(), TIME_T_OFFSET),
                abi::add_immediate(abi::ARG[1], abi::stack_pointer(), TM_OFFSET),
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
            // buffer (bug-42). The return sits in the return register, which is
            // also `RESULT_TAG_REGISTER`, so test it before the OK tail overwrites
            // it below.
            instructions.push(abi::compare_immediate(abi::RET[0], "0"));
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

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LR_OFFSET);
    Ok((frame, instructions, relocations, stack_slots))
}
