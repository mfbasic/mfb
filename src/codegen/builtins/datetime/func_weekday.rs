//! `datetime::weekday` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/weekday.md`.

use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str = r#"The day of the week of a `DateTime`'s civil date."#;
const DESC: &str = r#"`datetime::weekday` returns the day of the week on which `dt`'s civil date falls,
as a value of the `Weekday` enum (`Monday`, `Tuesday`, `Wednesday`, `Thursday`,
`Friday`, `Saturday`, `Sunday`).

The result is derived solely from the calendar date fields carried by `dt` — its
year, month, and day as stored in `dt`'s own zone. The day count for that civil
date is computed on the proleptic-Gregorian calendar and reduced modulo seven
against a fixed reference (`floorMod(days + 3, 7)`), so the answer is the
wall-clock weekday a person reading `dt`'s date in its zone would name. The
time-of-day fields, the sub-second nanoseconds, and the zone's UTC offset do not
affect the result; no `Instant` is resolved and no zone table is consulted.


Because the computation reads only `dt`'s stored civil date, the same instant
projected into two different zones can report two different weekdays whenever the
zones place that instant on opposite sides of midnight. The week is treated as
starting on Monday, matching the ordering of the `Weekday` enum, so
`Weekday.Monday` is the first day and `Weekday.Sunday` is the last.


`datetime::weekday` is pure: it reads no host state and has no side effects."#;
const EX: &str = r#"Name the weekday of a civil date in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
  LET w AS Weekday = datetime::weekday(dt)
END SUB
```

Branch on whether a `DateTime` falls on the weekend:

```
IMPORT datetime
IMPORT io

SUB main()
  LET dt AS DateTime = datetime::civil(datetime::date(2026, 6, 26), datetime::time(9, 30), datetime::local())
  LET w AS Weekday = datetime::weekday(dt)
  IF w = Weekday.Saturday OR w = Weekday.Sunday THEN
    io::print("weekend")
  END IF
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_weekday(dt AS DateTime) AS Weekday
  LET days AS Integer = __datetime_daysFromCivil(dt.date.year, dt.date.month, dt.date.day)
  LET idx AS Integer = __datetime_floorMod(days + 3, 7)
  IF idx = 0 THEN RETURN Weekday.Monday
  IF idx = 1 THEN RETURN Weekday.Tuesday
  IF idx = 2 THEN RETURN Weekday.Wednesday
  IF idx = 3 THEN RETURN Weekday.Thursday
  IF idx = 4 THEN RETURN Weekday.Friday
  IF idx = 5 THEN RETURN Weekday.Saturday
  RETURN Weekday.Sunday
END FUNC"#;

pub(crate) const WEEKDAY: BuiltinFunction = BuiltinFunction::mfb(
    "datetime.weekday",
    "weekday",
    INTRO,
    DESC,
    &[],
    &[super::ov(&[super::req("dt", "DateTime")], "Weekday")],
    BODY,
)
.with_example(EX);
