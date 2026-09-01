//! `datetime::monotonicNanos` — descriptor entry + authored docs, and the
//! per-member `abi_function` lowering ([`lower_monotonic_nanos`]). The wrapper
//! finalizes it (crypto/io's clean-room shape).

use super::gen_shared::{
    emit_libc_clock_nanos, void_int_result, CLOCK_MONOTONIC_DARWIN, CLOCK_MONOTONIC_LINUX,
    LOCALS_SIZE, NANOS_PER_SEC, WIN_FILETIME_OFFSET, WIN_QPC_FREQ_OFFSET,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// `abi_function` body for `datetime::monotonicNanos` — a monotonic-clock reading
/// in nanoseconds. On libc it rides the shared [`emit_libc_clock_nanos`] with the
/// platform's `CLOCK_MONOTONIC` id (Linux 1 / Darwin 6); on Windows it converts a
/// `QueryPerformanceCounter` tick count with an overflow-safe tick→nanosecond fold
/// (plan-66-A). Always succeeds.
pub(crate) fn lower_monotonic_nanos(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();

    if platform.family() == PlatformFamily::Windows {
        // QueryPerformanceCounter(&counter); QueryPerformanceFrequency(&freq).
        instructions.push(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            WIN_FILETIME_OFFSET,
        ));
        platform.emit_external_call(
            "QueryPerformanceCounter",
            &symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            WIN_QPC_FREQ_OFFSET,
        ));
        platform.emit_external_call(
            "QueryPerformanceFrequency",
            &symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // nanos = (counter/freq)*1e9 + ((counter%freq)*1e9)/freq. Splitting the
        // multiply across the quotient and remainder keeps every intermediate
        // inside u64: `counter*1e9` alone overflows within ~21 s at 10 MHz.
        let counter = vregs.next();
        let freq = vregs.next();
        let q = vregs.next();
        let rem = vregs.next();
        let scale = vregs.next();
        instructions.extend([
            abi::load_u64(&counter, abi::stack_pointer(), WIN_FILETIME_OFFSET), // counter
            abi::load_u64(&freq, abi::stack_pointer(), WIN_QPC_FREQ_OFFSET),    // freq
            abi::unsigned_divide_registers(&q, &counter, &freq),                // q
            abi::multiply_subtract_registers(&rem, &q, &freq, &counter), // rem = counter - q*freq
            abi::move_immediate(&scale, "Integer", NANOS_PER_SEC),
            abi::multiply_registers(&q, &q, &scale), // q*1e9
            abi::multiply_registers(&rem, &rem, &scale), // rem*1e9
            abi::unsigned_divide_registers(&rem, &rem, &freq), // (rem*1e9)/freq
            abi::add_registers(RESULT_VALUE_REGISTER, &q, &rem),
        ]);
    } else {
        let clock_id = match platform.family() {
            PlatformFamily::MacOS => CLOCK_MONOTONIC_DARWIN,
            PlatformFamily::Linux => CLOCK_MONOTONIC_LINUX,
            // Windows is routed above and never reaches this libc clock-id selection.
            PlatformFamily::Windows => {
                unreachable!("plan-66-A routes Windows datetime to kernel32")
            }
        };
        emit_libc_clock_nanos(
            clock_id,
            &symbol,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        )?;
    }

    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = LOCALS_SIZE;
    Ok(void_int_result("datetime.monotonicNanos"))
}

const INTRO: &str = r#"The raw monotonic-clock reading as a whole nanosecond count."#;
const DESC: &str = r#"`datetime::monotonicNanos` reads the host's monotonic clock and returns the
elapsed time, in whole nanoseconds, from an arbitrary fixed origin chosen by the
operating system. It is the low-level OS-seam intrinsic that backs
`datetime::monotonic`: where `monotonic` packages the reading into a `datetime::Duration`,
`monotonicNanos` returns the same value as a single raw `Integer` count of
nanoseconds.

The clock never moves backward: a later call always returns a value that is
greater than or equal to an earlier one. The reading is unrelated to wall-clock
time, carries no calendar meaning, and is not comparable across processes or
across reboots, so the absolute value of a single reading is meaningless. The
only intended use is to measure elapsed time: take two readings and subtract the
earlier from the later, yielding an elapsed interval in nanoseconds.

Because the clock is immune to wall-clock adjustments (NTP steps, manual clock
changes, daylight saving), the difference between two readings is a reliable
interval where a difference of `datetime::nowNanos` readings would not be. Use
the wall-clock readings, not the monotonic ones, whenever you need an actual
point in time.

The reading is a single nanoseconds-since-origin count taken from the host.
Prefer `datetime::monotonic` in ordinary code; reach
for `monotonicNanos` only when you want the bare integer count without
constructing a `datetime::Duration`.

`monotonicNanos` is **not pure**: two calls may return different values, and the
values depend on host clock state. It takes no arguments, reads clock state only,
and has no side effects. The reading always succeeds and never raises an
error."#;
const EX: &str = r#"Measure the elapsed time around a block of work in nanoseconds:

```
IMPORT datetime

SUB main()
  LET t0 AS Integer = datetime::monotonicNanos()
  ' ... work ...
  LET elapsedNanos AS Integer = datetime::monotonicNanos() - t0
END SUB
```

Convert the measured interval to whole milliseconds:

```
IMPORT datetime

SUB main()
  LET t0 AS Integer = datetime::monotonicNanos()
  ' ... work ...
  LET elapsedMs AS Integer = (datetime::monotonicNanos() - t0) / 1000000
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "monotonicNanos",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::Integer,
            errors: vec![],
            body: super::Body::abi_function(lower_monotonic_nanos),
        }],
    });
}
