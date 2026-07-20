# dayOfYear

The ordinal day within the year of a `DateTime`'s civil date.

## Synopsis

```
datetime::dayOfYear(dt AS DateTime) AS Integer
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

`datetime::dayOfYear` returns the ordinal position of `dt`'s civil date within
its calendar year: `1` for January 1, `2` for January 2, and so on through `365`
in a common year or `366` in a leap year (the value reached on December 31).
[[src/builtins/datetime.rs:call_return_type_name]]

The result is derived solely from the calendar date fields carried by `dt` — its
year, month, and day as stored in `dt`'s own zone. The day-of-year is computed on
the proleptic-Gregorian calendar by taking the days-from-civil count of `dt`'s
date, subtracting the days-from-civil count of January 1 of the same year, and
adding one (`here - start + 1`), so leap years correctly extend the count past
February. The time-of-day fields, the sub-second nanoseconds, and the zone's UTC
offset do not affect the result; no `Instant` is resolved and no zone table is
consulted. [[src/builtins/datetime_package.mfb:__datetime_dayOfYear]]

Because the computation reads only `dt`'s stored civil date, the same instant
projected into two different zones can report two different day-of-year values
whenever the zones place that instant on opposite sides of midnight, and across
the December 31 / January 1 boundary the two zones can even fall in different
years. [[src/builtins/datetime_package.mfb:__datetime_dayOfYear]]

`datetime::dayOfYear` is pure: it reads no host state and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The `DateTime` whose civil date is examined. Only the date's year, month, and day are used; the time-of-day, nanoseconds, and zone offset are ignored when computing the ordinal day. [[src/builtins/datetime_package.mfb:__datetime_dayOfYear]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The 1-based ordinal day of `dt`'s civil date within its year, from `1` for January 1 through `365` (common year) or `366` (leap year) for December 31. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Find the day-of-year of a civil date in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
  LET n AS Integer = datetime::dayOfYear(dt)
END SUB
```

Compute how many days remain in the year:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::civil(datetime::date(2026, 6, 26), datetime::time(9, 30), datetime::local())
  MUT total AS Integer = 365
  IF datetime::isLeapYear(dt.date.year) THEN
    total = 366
  END IF
  LET remaining AS Integer = total - datetime::dayOfYear(dt)
END SUB
```

## See also

- `mfb man datetime weekday`
- `mfb man datetime isLeapYear`
- `mfb man datetime daysInMonth`
- `mfb man datetime civil`
