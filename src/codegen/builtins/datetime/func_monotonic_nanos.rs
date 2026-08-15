//! `datetime::monotonicNanos` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/monotonicNanos.md`.

const INTRO: &str = r#"The raw monotonic-clock reading as a whole nanosecond count."#;
const DESC: &str = r#"`datetime::monotonicNanos` reads the host's monotonic clock and returns the
elapsed time, in whole nanoseconds, from an arbitrary fixed origin chosen by the
operating system. It is the low-level OS-seam intrinsic that backs
`datetime::monotonic`: where `monotonic` packages the reading into a `Duration`,
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

Internally the call lowers to a libc runtime helper that reads a single
nanoseconds-since-origin value from the OS (`clock_gettime(CLOCK_MONOTONIC)` on
the supported platforms). Prefer `datetime::monotonic` in ordinary code; reach
for `monotonicNanos` only when you want the bare integer count without
constructing a `Duration`.

`monotonicNanos` is **not pure**: two calls may return different values, and the
values depend on host clock state. It takes no arguments, reads clock state only,
and has no side effects. The reading always succeeds — the intrinsic returns an
`Integer` in the result register with the OK tag set and never raises an error."#;
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

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "monotonicNanos",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: super::arg_hint("monotonicNanos"),
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::Integer,
            errors: vec![],
            lowering: super::Lowering::Helper,
            body: super::Body::native(
                Some(super::lower_datetime_helper),
                Some(super::lower_datetime_helper),
                None,
            ),
        }],
    });
}
