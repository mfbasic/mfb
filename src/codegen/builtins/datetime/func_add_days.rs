//! `datetime::addDays` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/addDays.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Shift a civil `DateTime` by a whole number of calendar days, preserving its wall-clock time and zone."#;
const DESC: &str = r#"`datetime::addDays` advances `dt` by a whole number of calendar days and returns
the resulting `DateTime`. It converts `dt`'s calendar date to a serial day count,
adds `days`, converts that count back to a year-month-day date, and rebuilds the
`DateTime` from the new date, `dt`'s original wall-clock time, and `dt`'s original
zone.

Because the result is re-resolved through `dt`'s zone, `addDays` is
daylight-saving aware: the wall-clock time of day is preserved and the UTC offset
is recomputed for the new date, so crossing a DST transition shifts the
underlying instant by the appropriate 23-, 24-, or 25-hour day rather than a
fixed `86_400` seconds. The sub-second nanosecond component of the time is carried
through unchanged.

`days` is a signed count: a positive value moves `dt` later in the calendar and a
negative value moves it earlier. Adding zero days returns a `DateTime` equal to
`dt`. The operation works purely in whole days and never alters the hour, minute,
second, or nanosecond fields; for month-length-aware shifts use
`datetime::addMonths`, and for uniform physical-time arithmetic on an `Instant`
use `datetime::add`. `addDays` is pure: the same `DateTime` and day count always
yield the same result, and it has no side effects."#;
const EX: &str = r#"Advance a `DateTime` by one week:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET nextWeek AS DateTime = datetime::addDays(dt, 7)
END SUB
```

A negative count moves the date earlier:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET yesterday AS DateTime = datetime::addDays(dt, -1)
END SUB
```"#;

pub(crate) const ADD_DAYS: BuiltinFunction = BuiltinFunction::custom(
    "datetime.addDays",
    "addDays",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[super::req("dt", "DateTime"), super::req("days", super::I)],
        "DateTime",
    )],
)
.with_example(EX);
