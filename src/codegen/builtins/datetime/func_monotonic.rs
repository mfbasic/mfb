//! `datetime::monotonic` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"A monotonically non-decreasing clock reading for measuring elapsed time."#;
const DESC: &str = r#"`datetime::monotonic` reads the host's monotonic clock and returns the elapsed
span, as a `datetime::Duration`, from an arbitrary fixed origin chosen by the operating
system. The clock never moves backward: a later call always returns a span that
is greater than or equal to an earlier one. It is unrelated to wall-clock time,
carries no calendar meaning, and resets when the host reboots, so a single reading is not a
timestamp. Readings taken by different processes on the same host share one
origin on macOS.

The only intended use is to measure elapsed time: take two readings and subtract
the earlier from the later with `datetime::minus`. Because the clock is immune to
wall-clock adjustments (NTP steps, manual clock changes), the
difference is a reliable interval where `datetime::now` would not be. Use
`datetime::now`, not `monotonic`, whenever you need an actual point in time.

`monotonic` is `datetime::monotonicNanos` split into the `seconds` and `nanos`
fields of a `datetime::Duration`. The split never fails, and `nanos` always falls in
`0 .. 999_999_999`. The reading itself shares `monotonicNanos`'s limit: a
nanosecond count that does not fit an `Integer` raises `ErrOverflow`.


`monotonic` is **not pure**: two calls may return different spans, and the values
depend on host clock state. It takes no arguments, reads clock state only, and
has no side effects."#;
const EX: &str = r#"Measure the elapsed time around a block of work:

```
IMPORT datetime

SUB main()
  LET t0 AS datetime::Duration = datetime::monotonic()
  ' ... work ...
  LET elapsed AS datetime::Duration = datetime::minus(datetime::monotonic(), t0)
END SUB
```

Render the measured interval as text:

```
IMPORT datetime

SUB main()
  LET t0 AS datetime::Duration = datetime::monotonic()
  ' ... work ...
  LET span AS datetime::Duration = datetime::minus(datetime::monotonic(), t0)
  LET text AS String = datetime::formatDuration(span)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_monotonic AS Duration
  LET ns AS Integer = datetime::monotonicNanos()
  RETURN __datetime_normDuration(ns / 1000000000, ns MOD 1000000000)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "monotonic",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::named("Duration"),
            errors: vec!["ErrOverflow"],
            body: super::Body::mfb(BODY, "__datetime_monotonic"),
        }],
    });
}
