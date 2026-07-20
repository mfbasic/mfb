# subtract

Shift an `Instant` backward along the UTC timeline by a `Duration`.

## Synopsis

```
datetime::subtract(at AS Instant, by AS Duration) AS Instant
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

`datetime::subtract` returns the `Instant` reached by moving `at` backward along
the UTC timeline by the span `by`. It subtracts the `seconds` field of `by` from
the `seconds` field of `at` and the `nanos` field of `by` from the `nanos` field
of `at`, independently, then normalizes the difference so the stored `nanos`
lands in the range `0 .. 999_999_999`, borrowing a whole second from the
`seconds` field when the nanosecond difference is negative. The result is a point
on the same Unix-epoch, leap-second-free timeline as `at`.
[[src/builtins/datetime_package.mfb:__datetime_subtract]]

Because `by` is a signed `Duration`, `subtract` covers both directions on the
timeline: a positive span moves the `Instant` earlier and a negative span moves
it later, so `datetime::add(at, by)` and `datetime::subtract(at, by)` name
opposite shifts. The arithmetic is uniform second-and-nanosecond subtraction with
no awareness of calendars, time zones, or daylight-saving transitions; it simply
counts elapsed physical time. For civil, zone-aware day and month arithmetic that
honors DST and varying month lengths, use `datetime::addDays` and
`datetime::addMonths` on a `DateTime` instead.

Normalization floor-divides the nanosecond difference into a whole-second borrow
and a non-negative remainder, then folds the borrow back into the `seconds`
field, so a subtraction that borrows across the second boundary still yields a
`nanos` in `0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_normInstant]]
The subtraction is ordinary signed `Integer` arithmetic, so a span large enough
to push the combined second count past the `Integer` range overflows and traps.
`subtract` is pure: the same `Instant` and `Duration` always yield the same
`Instant`, and it has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `at` | `Instant` | The starting point on the UTC timeline to shift. Its `seconds` field is whole seconds since `1970-01-01T00:00:00Z` (possibly negative for instants before the epoch) and its `nanos` field is the sub-second remainder. [[src/builtins/datetime.rs:SUBTRACT]] |
| `by` | `Duration` | The signed span to subtract. A positive `Duration` moves `at` to an earlier `Instant`; a negative `Duration` advances it to a later one. Its `seconds` and `nanos` fields are subtracted from those of `at`. |

## Return value

| Type | Description |
| --- | --- |
| `Instant` | The `Instant` at `at - by`, normalized so its `seconds` field holds the whole-second count (which may be negative) and its `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Subtracting the `seconds` fields, or borrowing the normalized nanoseconds from the `seconds` field, produces a value outside the signed `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Move an `Instant` back by a 90-second span:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET earlier AS Instant = datetime::subtract(base, datetime::duration(90))
END SUB
```

A negative `Duration` shifts the `Instant` forward:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET later AS Instant = datetime::subtract(base, datetime::duration(-3600))
END SUB
```

## See also

- `mfb man datetime add`
- `mfb man datetime between`
- `mfb man datetime duration`
- `mfb man datetime addDays`
- `mfb man datetime addMonths`
