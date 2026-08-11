//! `datetime::addMonths` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/addMonths.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Shift a civil `DateTime` by a whole number of calendar months, clamping the day-of-month to the target month's length."#;
const DESC: &str = r#"`datetime::addMonths` advances `dt` by a whole number of calendar months and
returns the resulting `DateTime`. It collapses `dt`'s year and month into a
single month index (`year * 12 + month - 1`), adds `months`, and splits the sum
back into a target year and month with a flooring divide so that crossing year
boundaries in either direction is handled correctly.
 The wall-clock time of day
and the zone are taken unchanged from `dt`, and the result is re-resolved through
`dt`'s zone so the UTC offset is recomputed for the new date.


Because months vary in length, the day of month is clamped to the number of days
in the target month. If `dt`'s day-of-month exceeds the target month's length the
result lands on the last day of that month, so January 31 plus one month is
February 28 (or February 29 in a leap year), and any earlier day is preserved
exactly. The day is never carried over into the following month.


`months` is a signed count: a positive value moves `dt` later in the calendar and
a negative value moves it earlier; adding zero months returns a `DateTime` with
the same date as `dt`. The operation works purely in whole months and never
alters the hour, minute, second, or nanosecond fields; the sub-second nanosecond
component is carried through unchanged. Because the result is re-resolved through
`dt`'s zone, `addMonths` is daylight-saving aware: the wall-clock time is
preserved while the underlying instant absorbs any offset change for the new
date. For whole-day shifts use `datetime::addDays`, and for uniform physical-time
arithmetic on an `Instant` use `datetime::add`. `addMonths` is pure: the same
`DateTime` and month count always yield the same result, and it has no side
effects."#;
const EX: &str = r#"Advance a `DateTime` by one month:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET nextMonth AS DateTime = datetime::addMonths(dt, 1)
END SUB
```

A negative count moves the date earlier, and an overlong day clamps to the end of
the shorter month:

```
IMPORT datetime

SUB main()
  LET jan31 AS DateTime = datetime::civil(datetime::date(2025, 1, 31), datetime::time(9, 0, 0), datetime::utc())
  LET feb28 AS DateTime = datetime::addMonths(jan31, 1)
  LET lastYear AS DateTime = datetime::addMonths(jan31, -12)
END SUB
```"#;

pub(crate) const ADD_MONTHS: BuiltinFunction = BuiltinFunction::custom(
    "datetime.addMonths",
    "addMonths",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[super::req("dt", "DateTime"), super::req("months", super::I)],
        "DateTime",
    )],
)
.with_example(EX);
