//! `datetime::monotonic` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/monotonic.md`.

const INTRO: &str = r#"A monotonically non-decreasing clock reading for measuring elapsed time."#;
const DESC: &str = r#"`datetime::monotonic` reads the host's monotonic clock and returns the elapsed
span, as a `Duration`, from an arbitrary fixed origin chosen by the operating
system. The clock never moves backward: a later call always returns a span that
is greater than or equal to an earlier one. It is unrelated to wall-clock time,
carries no calendar meaning, and is not comparable across processes or across
reboots, so the absolute value of a single reading is meaningless.

The only intended use is to measure elapsed time: take two readings and subtract
the earlier from the later with `datetime::minus`. Because the clock is immune to
wall-clock adjustments (NTP steps, manual clock changes, daylight saving), the
difference is a reliable interval where `datetime::now` would not be. Use
`datetime::now`, not `monotonic`, whenever you need an actual point in time.

Internally `monotonic` reads a single nanoseconds-since-origin value from the OS
intrinsic (`datetime::monotonicNanos`, `clock_gettime(CLOCK_MONOTONIC)` on the
supported platforms), then splits it into the `seconds` and `nanos` fields of a
`Duration` by a truncating divide and remainder against `1_000_000_000`. The
divisor is a non-zero constant, so the split cannot trap, and the nanosecond
remainder already falls in `0 .. 999_999_999`.


`monotonic` is **not pure**: two calls may return different spans, and the values
depend on host clock state. It takes no arguments, reads clock state only, and
has no side effects."#;
const EX: &str = r#"Measure the elapsed time around a block of work:

```
IMPORT datetime

SUB main()
  LET t0 AS Duration = datetime::monotonic()
  ' ... work ...
  LET elapsed AS Duration = datetime::minus(datetime::monotonic(), t0)
END SUB
```

Render the measured interval as text:

```
IMPORT datetime

SUB main()
  LET t0 AS Duration = datetime::monotonic()
  ' ... work ...
  LET span AS Duration = datetime::minus(datetime::monotonic(), t0)
  LET text AS String = datetime::formatDuration(span)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_monotonic AS Duration
  LET ns AS Integer = datetime::monotonicNanos()
  RETURN __datetime_normDuration(ns / 1000000000, ns MOD 1000000000)
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "monotonic",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: super::arg_hint("monotonic"),
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::Named("Duration"),
            errors: vec![],
            lowering: super::Lowering::Helper,
            body: super::Body::mfb(BODY, "__datetime_monotonic"),
        }],
    });
}
