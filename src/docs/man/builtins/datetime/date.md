# date

Validate and build a calendar `Date` from year, month, and day components.

## Synopsis

```
datetime::date(year AS Integer, month AS Integer, day AS Integer) AS Date
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

`datetime::date` builds a calendar `Date` on the proleptic-Gregorian calendar
from its `year`, `month`, and `day` components. The calendar is *proleptic*: the
Gregorian rules are extended uniformly to every year, including those before the
calendar's historical adoption. `year` is an unrestricted `Integer` and may be
zero or negative. [[src/builtins/datetime_package.mfb:__datetime_date]]

The constructor validates the date before returning it. `month` must name a real
month in `1 .. 12`, and `day` must be in range for that month and year. The upper
bound on `day` is the actual length of the given month, computed the same way as
`datetime::daysInMonth`, so it depends on both `month` and `year`: April allows
`1 .. 30`, and February allows `1 .. 29` only in a leap year and `1 .. 28`
otherwise. February 29 is therefore accepted in leap years such as 2024 and
rejected in common years such as 2026. There is no normalization or wrap-around:
an out-of-range component is an error, not silently carried into the next unit.
[[src/builtins/datetime_package.mfb:__datetime_date]]

`date` is pure: the same arguments always yield the same `Date`, and it has no
side effects. A `Date` carries only calendar fields and no zone or time-of-day;
pair it with `datetime::time` and `datetime::civil` to build a zoned `DateTime`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `year` | `Integer` | The proleptic-Gregorian year. Unrestricted: any `Integer` is accepted, including zero and negative values. Participates in the leap-year determination for the `day` range check. |
| `month` | `Integer` | The month of the year. Must be in `1 .. 12`, where `1` is January and `12` is December. Any value outside this range is an error. [[src/builtins/datetime_package.mfb:__datetime_date]] |
| `day` | `Integer` | The day of the month. Must be in `1 .. N`, where `N` is the number of days in the given month and year (28, 29, 30, or 31). Any value outside this range is an error. [[src/builtins/datetime_package.mfb:__datetime_date]] |

## Return value

| Type | Description |
| --- | --- |
| `Date` | A `Date` holding the validated `year`, `month`, and `day`. Returned only when all three components form a real calendar date. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `month` is outside `1 .. 12`, or `day` is outside the valid range for the given month and year (for example `datetime::date(2026, 2, 29)`, since 2026 is not a leap year). [[src/builtins/datetime_package.mfb:__datetime_date]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Construct a valid date:

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
```

## See also

- `mfb man datetime time`
- `mfb man datetime civil`
- `mfb man datetime daysInMonth`
- `mfb man datetime isLeapYear`
