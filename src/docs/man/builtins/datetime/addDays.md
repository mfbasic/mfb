# addDays

Shift a civil `DateTime` by a whole number of calendar days, preserving its wall-clock time and zone.

## Synopsis

```
datetime::addDays(dt AS DateTime, days AS Integer) AS DateTime
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

`datetime::addDays` advances `dt` by a whole number of calendar days and returns
the resulting `DateTime`. It converts `dt`'s calendar date to a serial day count,
adds `days`, converts that count back to a year-month-day date, and rebuilds the
`DateTime` from the new date, `dt`'s original wall-clock time, and `dt`'s original
zone. [[src/builtins/datetime_package.mfb:__datetime_addDays]]

Because the result is re-resolved through `dt`'s zone, `addDays` is
daylight-saving aware: the wall-clock time of day is preserved and the UTC offset
is recomputed for the new date, so crossing a DST transition shifts the
underlying instant by the appropriate 23-, 24-, or 25-hour day rather than a
fixed `86_400` seconds. The sub-second nanosecond component of the time is carried
through unchanged. [[src/builtins/datetime_package.mfb:__datetime_resolveLocal]] [[src/builtins/datetime_package.mfb:__datetime_civil]]

`days` is a signed count: a positive value moves `dt` later in the calendar and a
negative value moves it earlier. Adding zero days returns a `DateTime` equal to
`dt`. The operation works purely in whole days and never alters the hour, minute,
second, or nanosecond fields; for month-length-aware shifts use
`datetime::addMonths`, and for uniform physical-time arithmetic on an `Instant`
use `datetime::add`. `addDays` is pure: the same `DateTime` and day count always
yield the same result, and it has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The civil starting point. Its date, wall-clock time, and zone are read; the date is shifted while the time of day and zone are preserved. [[src/builtins/datetime.rs:ADD_DAYS]] |
| `days` | `Integer` | The signed number of whole calendar days to add. Positive values advance `dt` to a later date, negative values to an earlier one, and zero leaves the date unchanged. |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | The `DateTime` `dt` shifted by `days` calendar days, holding the new date, `dt`'s original time of day (re-resolved against `dt`'s zone for the new offset), and `dt`'s original zone. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The day-count arithmetic (`daysFromCivil(...) + days`), or the conversion of the shifted date back to epoch seconds during zone resolution, produces a value outside the signed `Integer` range. [[src/builtins/datetime_package.mfb:__datetime_addDays]] [[src/builtins/datetime_package.mfb:__datetime_civil]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Advance a `DateTime` by one week:

```
IMPORT datetime

LET dt AS DateTime = datetime::toUtc(datetime::now())
LET nextWeek AS DateTime = datetime::addDays(dt, 7)
```

A negative count moves the date earlier:

```
IMPORT datetime

LET dt AS DateTime = datetime::toUtc(datetime::now())
LET yesterday AS DateTime = datetime::addDays(dt, -1)
```

## See also

- `mfb man datetime addMonths`
- `mfb man datetime add`
- `mfb man datetime startOfDay`
- `mfb man datetime weekday`
