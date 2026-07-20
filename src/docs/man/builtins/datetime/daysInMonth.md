# daysInMonth

The number of days in a calendar month.

## Synopsis

```
datetime::daysInMonth(year AS Integer, month AS Integer) AS Integer
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

`datetime::daysInMonth` returns the number of days in the given `month` of the
given `year` under the proleptic-Gregorian calendar. The result is `31` for
January, March, May, July, August, October, and December; `30` for April, June,
September, and November; and `28` or `29` for February depending on whether
`year` is a leap year. [[src/builtins/datetime_package.mfb:__datetime_daysInMonth]]

February's length is decided by applying the leap-year rule to `year`: a leap
February has `29` days, otherwise it has `28`. The leap rule is purely
arithmetic on the year number (divisible by `4`, except century years that are
not divisible by `400`), so it extends indefinitely into the past and future and
treats zero and negative year numbers by the same divisibility test.
[[src/builtins/datetime_package.mfb:__datetime_isLeapYear]]

Only February consults `year`; for every other month the result depends solely
on `month`, and `year` is ignored. The `month` argument is not range-checked:
any value that is not `2`, `4`, `6`, `9`, or `11` yields `31`, so out-of-range
month numbers do not raise an error but return `31` by falling through to the
default case. [[src/builtins/datetime_package.mfb:__datetime_daysInMonth]]

The function reads no time zone, `Instant`, or current clock value and has no
side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `year` | `Integer` | The proleptic-Gregorian year number. It affects the result only when `month` is `2`, where it selects February's length via the leap-year rule. Any `Integer` is accepted, including zero and negative values. [[src/builtins/datetime.rs:call_param_names]] |
| `month` | `Integer` | The month of the year, where `1` is January and `12` is December. Values `2`, `4`, `6`, `9`, and `11` select February (28 or 29), April, June, September, and November (30) respectively; every other value, including out-of-range numbers, returns `31`. [[src/builtins/datetime.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of days in `month` of `year`: `31`, `30`, or (for February) `29` in a leap year and `28` otherwise. Any `month` value outside `1 .. 12` returns `31`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Length of common and leap-year February:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::daysInMonth(2023, 2)))   ' 28
  io::print(toString(datetime::daysInMonth(2024, 2)))   ' 29 (leap year)
END SUB
```

Lengths of the other months ignore the year:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::daysInMonth(2026, 1)))   ' 31
  io::print(toString(datetime::daysInMonth(2026, 4)))   ' 30
END SUB
```

Clamp a day-of-month to the end of its month:

```
IMPORT datetime
IMPORT io

SUB main()
  LET year AS Integer = 2024
  LET month AS Integer = 2
  MUT day AS Integer = 31
  LET last AS Integer = datetime::daysInMonth(year, month)
  IF day > last THEN
    day = last
  END IF
  io::print(toString(day))   ' 29
END SUB
```

## See also

- `mfb man datetime isLeapYear`
- `mfb man datetime date`
- `mfb man datetime dayOfYear`
