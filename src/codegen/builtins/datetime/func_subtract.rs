//! `datetime::subtract` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/subtract.md`.

use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = r#"Shift an `Instant` backward along the UTC timeline by a `Duration`."#;
const DESC: &str = r#"`datetime::subtract` returns the `Instant` reached by moving `at` backward along
the UTC timeline by the span `by`. It subtracts the `seconds` field of `by` from
the `seconds` field of `at` and the `nanos` field of `by` from the `nanos` field
of `at`, independently, then normalizes the difference so the stored `nanos`
lands in the range `0 .. 999_999_999`, borrowing a whole second from the
`seconds` field when the nanosecond difference is negative. The result is a point
on the same Unix-epoch, leap-second-free timeline as `at`.


Because `by` is a signed `Duration`, `subtract` covers both directions on the
timeline: a positive span moves the `Instant` earlier and a negative span moves
it later, so `datetime::add(at, by)` and `datetime::subtract(at, by)` name
opposite shifts. The arithmetic is uniform second-and-nanosecond subtraction with
no awareness of calendars, time zones, or daylight-saving transitions; it simply
counts elapsed physical time. For civil, zone-aware day and month arithmetic that
honors DST and varying month lengths, use `datetime::addDays` and
`datetime::addMonths` on a `DateTime` instead.

Normalization floor-divides the nanosecond difference into a whole-second borrow
and a non-negative remainder, then folds the borrow back into the `seconds`
field, so a subtraction that borrows across the second boundary still yields a
`nanos` in `0 .. 999_999_999`.
The subtraction is ordinary signed `Integer` arithmetic, so a span large enough
to push the combined second count past the `Integer` range overflows and traps.
`subtract` is pure: the same `Instant` and `Duration` always yield the same
`Instant`, and it has no side effects."#;
const EX: &str = r#"Move an `Instant` back by a 90-second span:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET earlier AS Instant = datetime::subtract(base, datetime::duration(90))
END SUB
```

A negative `Duration` shifts the `Instant` forward:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET later AS Instant = datetime::subtract(base, datetime::duration(-3600))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_subtract(at AS Instant, by AS Duration) AS Instant
  RETURN __datetime_normInstant(at.seconds - by.seconds, at.nanos - by.nanos)
END FUNC"#;

pub(crate) const SUBTRACT: BuiltinFunction = BuiltinFunction::mfb(
    "datetime.subtract",
    "subtract",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[super::req("at", "Instant"), super::req("by", "Duration")],
        "Instant",
    )],
    BODY,
)
.with_example(EX);
