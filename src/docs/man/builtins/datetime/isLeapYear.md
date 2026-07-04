# isLeapYear

Whether a proleptic-Gregorian calendar year is a leap year.

## Synopsis

```
datetime::isLeapYear(year AS Integer) AS Boolean
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

`datetime::isLeapYear` applies the proleptic-Gregorian leap rule to `year` and
reports whether that year has 366 days. A year is a leap year when it is
divisible by 4, except for century years (those divisible by 100), which are
leap years only when they are also divisible by 400. So `2000` and `2024` are
leap years, while `1900` and `2023` are not.
[[src/builtins/datetime_package.mfb:__datetime_isLeapYear]]

The rule is purely arithmetic on the year number: no time zone, `Instant`, or
current clock value is consulted. The proleptic Gregorian calendar extends the
same rule indefinitely into the past and future, so years before the calendar's
historical adoption and negative (BCE-style) year numbers are evaluated by the
identical divisibility test on `4`, `100`, and `400`. The function reads no host
state and has no side effects. [[src/builtins/datetime_package.mfb:__datetime_isLeapYear]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `year` | `Integer` | The calendar year to test, interpreted as a proleptic-Gregorian year number. Any `Integer` is accepted, including zero and negative values; each is judged solely by its divisibility by `4`, `100`, and `400`. [[src/builtins/datetime.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `True` when `year` is a leap year (366 days, with a 29-day February) under the proleptic-Gregorian rule, and `False` otherwise. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Test individual years:

```
IMPORT datetime

PRINT datetime::isLeapYear(2000)   ' True  (divisible by 400)
PRINT datetime::isLeapYear(1900)   ' False (century, not /400)
PRINT datetime::isLeapYear(2024)   ' True  (divisible by 4)
PRINT datetime::isLeapYear(2023)   ' False
```

Pick February's length from the leap result:

```
IMPORT datetime

LET year AS Integer = 2024
LET days AS Integer = 28
IF datetime::isLeapYear(year) THEN
  LET days = 29
END IF
PRINT days   ' 29
```

## See also

- `mfb man datetime daysInMonth`
- `mfb man datetime dayOfYear`
- `mfb man datetime civil`
