//! `datetime::date` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/date.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Validate and build a calendar `Date` from year, month, and day components."#;
const DESC: &str = r#"`datetime::date` builds a calendar `Date` on the proleptic-Gregorian calendar
from its `year`, `month`, and `day` components. The calendar is *proleptic*: the
Gregorian rules are extended uniformly to every year, including those before the
calendar's historical adoption. `year` is an unrestricted `Integer` and may be
zero or negative.

The constructor validates the date before returning it. `month` must name a real
month in `1 .. 12`, and `day` must be in range for that month and year. The upper
bound on `day` is the actual length of the given month, computed the same way as
`datetime::daysInMonth`, so it depends on both `month` and `year`: April allows
`1 .. 30`, and February allows `1 .. 29` only in a leap year and `1 .. 28`
otherwise. February 29 is therefore accepted in leap years such as 2024 and
rejected in common years such as 2026. There is no normalization or wrap-around:
an out-of-range component is an error, not silently carried into the next unit.


`date` is pure: the same arguments always yield the same `Date`, and it has no
side effects. A `Date` carries only calendar fields and no zone or time-of-day;
pair it with `datetime::time` and `datetime::civil` to build a zoned `DateTime`."#;
const EX: &str = r#"Construct a valid date:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
END SUB
```

Combine a date and time into a zoned `DateTime`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET t AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, t, datetime::utc())
END SUB
```

An impossible calendar date raises `ErrInvalidArgument`:

```
IMPORT datetime

SUB main()
  LET bad AS Date = datetime::date(2026, 2, 29)
END SUB
```"#;

pub(crate) const DATE: BuiltinFunction = BuiltinFunction::custom(
    "datetime.date",
    "date",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[
            super::req("year", super::I),
            super::req("month", super::I),
            super::req("day", super::I),
        ],
        "Date",
    )],
)
.with_example(EX);
