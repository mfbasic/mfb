# weekday

The day of the week of a `DateTime`'s civil date.

## Synopsis

```
datetime::weekday(dt AS DateTime) AS Weekday
```

## Package

datetime

## Imports

```
IMPORT datetime
```

`datetime` is a built-in package, so no manifest dependency is required.
[[src/builtins/datetime.rs:augmented_project]]

## Description

`datetime::weekday` returns the day of the week on which `dt`'s civil date falls,
as a value of the `Weekday` enum (`Monday`, `Tuesday`, `Wednesday`, `Thursday`,
`Friday`, `Saturday`, `Sunday`). [[src/builtins/datetime.rs:call_return_type_name]]

The result is derived solely from the calendar date fields carried by `dt` — its
year, month, and day as stored in `dt`'s own zone. The day count for that civil
date is computed on the proleptic-Gregorian calendar and reduced modulo seven
against a fixed reference (`floorMod(days + 3, 7)`), so the answer is the
wall-clock weekday a person reading `dt`'s date in its zone would name. The
time-of-day fields, the sub-second nanoseconds, and the zone's UTC offset do not
affect the result; no `Instant` is resolved and no zone table is consulted.
[[src/builtins/datetime_package.mfb:__datetime_weekday]]

Because the computation reads only `dt`'s stored civil date, the same instant
projected into two different zones can report two different weekdays whenever the
zones place that instant on opposite sides of midnight. The week is treated as
starting on Monday, matching the ordering of the `Weekday` enum, so
`Weekday.Monday` is the first day and `Weekday.Sunday` is the last.
[[src/builtins/datetime_package.mfb:__datetime_weekday]]

`datetime::weekday` is pure: it reads no host state and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The `DateTime` whose civil date is examined. Only the date's year, month, and day are used; the time-of-day, nanoseconds, and zone offset are ignored for the purpose of naming the weekday. [[src/builtins/datetime_package.mfb:__datetime_weekday]] |

## Return value

| Type | Description |
| --- | --- |
| `Weekday` | The `Weekday` enum member naming the day of the week of `dt`'s civil date, one of `Weekday.Monday` through `Weekday.Sunday`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Name the weekday of a civil date in the local zone:

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
```

## See also

- `mfb man datetime dayOfYear`
- `mfb man datetime civil`
- `mfb man datetime format`
