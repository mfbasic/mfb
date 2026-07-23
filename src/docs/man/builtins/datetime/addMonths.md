# addMonths

Shift a civil `DateTime` by a whole number of calendar months, clamping the day-of-month to the target month's length.

## Synopsis

```
datetime::addMonths(dt AS DateTime, months AS Integer) AS DateTime
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

`datetime::addMonths` advances `dt` by a whole number of calendar months and
returns the resulting `DateTime`. It collapses `dt`'s year and month into a
single month index (`year * 12 + month - 1`), adds `months`, and splits the sum
back into a target year and month with a flooring divide so that crossing year
boundaries in either direction is handled correctly.
[[src/builtins/datetime_package.mfb:__datetime_floorDiv]] The wall-clock time of day
and the zone are taken unchanged from `dt`, and the result is re-resolved through
`dt`'s zone so the UTC offset is recomputed for the new date.
[[src/builtins/datetime_package.mfb:__datetime_addMonths]]

Because months vary in length, the day of month is clamped to the number of days
in the target month. If `dt`'s day-of-month exceeds the target month's length the
result lands on the last day of that month, so January 31 plus one month is
February 28 (or February 29 in a leap year), and any earlier day is preserved
exactly. The day is never carried over into the following month.
[[src/builtins/datetime_package.mfb:__datetime_daysInMonth]]

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
effects. [[src/builtins/datetime_package.mfb:__datetime_civil]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The civil starting point. Its date, wall-clock time, and zone are read; the year and month are shifted, the day-of-month is clamped to the target month's length, and the time of day and zone are preserved. [[src/builtins/datetime.rs:ADD_MONTHS]] |
| `months` | `Integer` | The signed number of whole calendar months to add. Positive values advance `dt` to a later month, negative values to an earlier one, and zero leaves the year and month unchanged. |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | The `DateTime` `dt` shifted by `months` calendar months, holding the new year and month, the original day-of-month clamped down to the target month's length when necessary, `dt`'s original time of day (re-resolved against `dt`'s zone for the new offset), and `dt`'s original zone. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The month-index arithmetic (`year * 12 + month - 1 + months`), or the conversion of the shifted date back to epoch seconds during zone resolution, produces a value outside the signed `Integer` range. [[src/builtins/datetime_package.mfb:__datetime_addMonths]] [[src/builtins/datetime_package.mfb:__datetime_civil]] [[src/target/shared/code/builder_error_emission.rs:emit_overflow_return]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Advance a `DateTime` by one month:

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
```

## See also

- `mfb man datetime addDays`
- `mfb man datetime add`
- `mfb man datetime daysInMonth`
- `mfb man datetime isLeapYear`
