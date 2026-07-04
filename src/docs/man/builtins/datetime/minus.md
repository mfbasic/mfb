# minus

Subtract one `Duration` span from another and return the resulting `Duration`.

## Synopsis

```
datetime::minus(a AS Duration, b AS Duration) AS Duration
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

`datetime::minus` returns the `Duration` `a - b`, the signed span left after
removing one span of elapsed physical time from another. It subtracts the
`seconds` field of `b` from the `seconds` field of `a` and the `nanos` field of
`b` from the `nanos` field of `a`, independently, then normalizes the result so
the stored `nanos` lands in the range `0 .. 999_999_999`, borrowing a whole
second from the `seconds` field when the nanosecond difference is negative.
[[src/builtins/datetime_package.mfb:__datetime_minus]]

Because both operands are signed `Duration`s, `minus` handles spans of either
direction: subtracting a negative `Duration` lengthens the total, and
subtracting a larger span from a smaller one yields a negative `Duration`.
`minus` pairs with `datetime::plus` and `datetime::negate`, since
`datetime::minus(a, b)` equals `datetime::plus(a, datetime::negate(b))`. A
common use is measuring elapsed time between two `datetime::monotonic` readings.

Normalization floor-divides the nanosecond difference into a whole-second borrow
and a non-negative remainder, then folds the borrow back into the `seconds`
field, so a `nanos` difference that goes negative still yields a `nanos` in
`0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_normDuration]]
The arithmetic is uniform second-and-nanosecond subtraction with no awareness of
calendars, time zones, or daylight-saving transitions; it simply differences
elapsed physical time. To shift a point on the timeline rather than combine two
spans, use `datetime::subtract` on an `Instant`. The subtraction is ordinary
signed `Integer` arithmetic, so a difference whose second count falls outside the
`Integer` range overflows and traps. `minus` is pure: the same two `Duration`s
always yield the same `Duration`, and it has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Duration` | The span subtracted from. Its `seconds` field is a whole-second count (which may be negative) and its `nanos` field is the sub-second remainder. [[src/builtins/datetime.rs:MINUS]] |
| `b` | `Duration` | The span to subtract. Its `seconds` and `nanos` fields are subtracted from those of `a`. A negative `Duration` adds to the running total. |

## Return value

| Type | Description |
| --- | --- |
| `Duration` | The `Duration` `a - b`, normalized so its `seconds` field holds the whole-second count (which may be negative) and its `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Subtracting the `seconds` fields, or borrowing the normalized nanoseconds from the `seconds` field, produces a value outside the signed `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Subtract a 500-millisecond span from a 90-second span:

```
IMPORT datetime

LET a AS Duration = datetime::duration(90)
LET b AS Duration = datetime::duration(0, 500_000_000)
LET rest AS Duration = datetime::minus(a, b)
```

Measure the elapsed time between two monotonic readings:

```
IMPORT datetime

LET start AS Duration = datetime::monotonic()
LET finish AS Duration = datetime::monotonic()
LET elapsed AS Duration = datetime::minus(finish, start)
```

## See also

- `mfb man datetime plus`
- `mfb man datetime negate`
- `mfb man datetime duration`
- `mfb man datetime subtract`
- `mfb man datetime monotonic`
