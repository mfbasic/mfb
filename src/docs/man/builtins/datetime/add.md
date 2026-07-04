# add

Shift an `Instant` forward along the UTC timeline by a `Duration`.

## Synopsis

```
datetime::add(at AS Instant, by AS Duration) AS Instant
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

`datetime::add` returns the `Instant` reached by advancing `at` forward along
the UTC timeline by the span `by`. It adds the two `seconds` fields and the two
`nanos` fields independently, then normalizes the sum so the stored `nanos`
lands in the range `0 .. 999_999_999`, carrying any whole seconds embedded in
the nanosecond sum into the `seconds` field. The result is a point on the same
Unix-epoch, leap-second-free timeline as `at`.
[[src/builtins/datetime_package.mfb:__datetime_add]]

Because `by` is a signed `Duration`, `add` covers both directions on the
timeline: a positive span moves the `Instant` later and a negative span moves it
earlier, so `datetime::add(at, by)` and `datetime::subtract(at, by)` name
opposite shifts. The arithmetic is uniform second-and-nanosecond addition with
no awareness of calendars, time zones, or daylight-saving transitions; it simply
counts elapsed physical time. For civil, zone-aware day and month arithmetic
that honors DST and varying month lengths, use `datetime::addDays` and
`datetime::addMonths` on a `DateTime` instead.

Normalization floor-divides the nanosecond sum into a whole-second carry and a
non-negative remainder, then folds the carry back into the `seconds` field, so
a negative `Duration` that borrows across the second boundary still yields a
`nanos` in `0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_normInstant]]
The addition is ordinary signed `Integer` arithmetic, so a span large enough to
push the combined second count past the `Integer` range overflows and traps.
`add` is pure: the same `Instant` and `Duration` always yield the same
`Instant`, and it has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `at` | `Instant` | The starting point on the UTC timeline to shift. Its `seconds` field is whole seconds since `1970-01-01T00:00:00Z` (possibly negative for instants before the epoch) and its `nanos` field is the sub-second remainder. [[src/builtins/datetime.rs:ADD]] |
| `by` | `Duration` | The signed span to add. A positive `Duration` advances `at` to a later `Instant`; a negative `Duration` moves it to an earlier one. Its `seconds` and `nanos` fields are added to those of `at`. |

## Return value

| Type | Description |
| --- | --- |
| `Instant` | The `Instant` at `at + by`, normalized so its `seconds` field holds the whole-second count (which may be negative) and its `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Adding the `seconds` fields, or carrying the normalized nanoseconds into the `seconds` field, produces a value outside the signed `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Advance an `Instant` by a 90-second span:

```
IMPORT datetime

LET base AS Instant = datetime::instant(1_700_000_000)
LET later AS Instant = datetime::add(base, datetime::duration(90))
```

A negative `Duration` shifts the `Instant` backward:

```
IMPORT datetime

LET base AS Instant = datetime::instant(1_700_000_000)
LET earlier AS Instant = datetime::add(base, datetime::duration(-3600))
```

## See also

- `mfb man datetime subtract`
- `mfb man datetime between`
- `mfb man datetime duration`
- `mfb man datetime addDays`
- `mfb man datetime addMonths`
