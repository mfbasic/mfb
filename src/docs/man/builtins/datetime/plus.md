# plus

Add two `Duration` spans into their combined `Duration`.

## Synopsis

```
datetime::plus(a AS Duration, b AS Duration) AS Duration
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

`datetime::plus` returns the `Duration` `a + b`, the signed span that results
from combining two spans of elapsed physical time. It adds the two `seconds`
fields and the two `nanos` fields independently, then normalizes the sum so the
stored `nanos` lands in the range `0 .. 999_999_999`, carrying any whole seconds
embedded in the nanosecond sum into the `seconds` field.
[[src/builtins/datetime_package.mfb:__datetime_plus]]

Because both operands are signed `Duration`s, `plus` handles spans of either
direction: adding a negative `Duration` shortens the total, and adding two
`Duration`s of opposite sign moves toward zero. The operation is commutative —
`datetime::plus(a, b)` and `datetime::plus(b, a)` yield the same `Duration` — and
pairs with `datetime::minus` and `datetime::negate`, since
`datetime::plus(a, datetime::negate(b))` equals `datetime::minus(a, b)`.

Normalization floor-divides the nanosecond sum into a whole-second carry and a
non-negative remainder, then folds the carry back into the `seconds` field, so a
combined `nanos` that overflows or goes negative still yields a `nanos` in
`0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_normDuration]]
The arithmetic is uniform second-and-nanosecond addition with no awareness of
calendars, time zones, or daylight-saving transitions; it simply totals elapsed
physical time. To shift a point on the timeline rather than combine two spans,
use `datetime::add` on an `Instant`. The addition is ordinary signed `Integer`
arithmetic, so a combined second count that exceeds the `Integer` range
overflows and traps. `plus` is pure: the same two `Duration`s always yield the
same `Duration`, and it has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Duration` | The first span to add. Its `seconds` field is a whole-second count (which may be negative) and its `nanos` field is the sub-second remainder. [[src/builtins/datetime.rs:PLUS]] |
| `b` | `Duration` | The second span to add. Its `seconds` and `nanos` fields are added to those of `a`. A negative `Duration` subtracts from the running total. |

## Return value

| Type | Description |
| --- | --- |
| `Duration` | The `Duration` `a + b`, normalized so its `seconds` field holds the whole-second count (which may be negative) and its `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Adding the `seconds` fields, or carrying the normalized nanoseconds into the `seconds` field, produces a value outside the signed `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Combine a 90-second span with a 500-millisecond span:

```
IMPORT datetime

LET a AS Duration = datetime::duration(90)
LET b AS Duration = datetime::duration(0, 500_000_000)
LET total AS Duration = datetime::plus(a, b)
```

Adding a negative `Duration` shortens the total:

```
IMPORT datetime

LET a AS Duration = datetime::duration(3600)
LET total AS Duration = datetime::plus(a, datetime::duration(-600))
```

## See also

- `mfb man datetime minus`
- `mfb man datetime negate`
- `mfb man datetime duration`
- `mfb man datetime add`
