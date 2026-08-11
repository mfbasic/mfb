//! `datetime::isLeapYear` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/isLeapYear.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Whether a proleptic-Gregorian calendar year is a leap year."#;
const DESC: &str = r#"`datetime::isLeapYear` applies the proleptic-Gregorian leap rule to `year` and
reports whether that year has 366 days. A year is a leap year when it is
divisible by 4, except for century years (those divisible by 100), which are
leap years only when they are also divisible by 400. So `2000` and `2024` are
leap years, while `1900` and `2023` are not.


The rule is purely arithmetic on the year number: no time zone, `Instant`, or
current clock value is consulted. The proleptic Gregorian calendar extends the
same rule indefinitely into the past and future, so years before the calendar's
historical adoption and negative (BCE-style) year numbers are evaluated by the
identical divisibility test on `4`, `100`, and `400`. The function reads no host
state and has no side effects."#;
const EX: &str = r#"Test individual years:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::isLeapYear(2000)))   ' True  (divisible by 400)
  io::print(toString(datetime::isLeapYear(1900)))   ' False (century, not /400)
  io::print(toString(datetime::isLeapYear(2024)))   ' True  (divisible by 4)
  io::print(toString(datetime::isLeapYear(2023)))   ' False
END SUB
```

Pick February's length from the leap result:

```
IMPORT datetime
IMPORT io

SUB main()
  LET year AS Integer = 2024
  MUT days AS Integer = 28
  IF datetime::isLeapYear(year) THEN
    days = 29
  END IF
  io::print(toString(days))   ' 29
END SUB
```"#;

pub(crate) const IS_LEAP_YEAR: BuiltinFunction = BuiltinFunction::custom(
    "datetime.isLeapYear",
    "isLeapYear",
    INTRO,
    DESC,
    &[],
    &[super::ov(&[super::req("year", super::I)], "Boolean")],
)
.with_example(EX);
