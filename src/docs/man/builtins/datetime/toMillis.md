# toMillis

Return the whole milliseconds between the Unix epoch and an `Instant`.

## Synopsis

```
datetime::toMillis(at AS Instant) AS Integer
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

`datetime::toMillis` collapses the absolute point `at` into a single `Integer`
count of whole milliseconds measured from the Unix epoch
(`1970-01-01T00:00:00Z`). Instants before the epoch yield negative counts, the
epoch itself yields `0`, and instants after the epoch yield positive counts.
[[src/builtins/datetime_package.mfb:__datetime_toMillis]]

The result is computed as `at.seconds * 1000 + at.nanos / 1000000`: the
seconds-since-epoch field is scaled to milliseconds and the sub-second `nanos`
field contributes its whole-millisecond part. The `nanos` division truncates,
discarding any sub-millisecond remainder (the microsecond and nanosecond
digits). Because a normalized `Instant` always holds a non-negative `nanos`
field in the range `0..999999999`, this truncation drops the fractional
millisecond rather than rounding it, in either direction.

The arithmetic is checked. For an instant near the extreme edge of the timeline
either the `at.seconds * 1000` scaling or the following addition can exceed the
signed `Integer` range, in which case the function raises `ErrOverflow` rather
than wrapping. `datetime::toMillis` is pure: it reads no host state and depends
only on `at`.

`datetime::toMillis` is the inverse of `datetime::fromMillis` to
whole-millisecond precision; sub-millisecond `nanos` in `at` are not recoverable
from the result. For full nanosecond precision use `datetime::toNanos`.
[[src/builtins/datetime_package.mfb:__datetime_fromMillis]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `at` | `Instant` | The absolute point on the UTC timeline to measure. Its `seconds` field (seconds since the Unix epoch, possibly negative) and its `nanos` field (`0..999999999`) together determine the millisecond count. Sub-millisecond `nanos` are truncated. [[src/builtins/datetime.rs:TO_MILLIS]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of whole milliseconds from the Unix epoch to `at`: negative before the epoch, `0` at the epoch, positive after. Any sub-millisecond fraction of `at` is discarded. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Evaluating `at.seconds * 1000 + at.nanos / 1000000` overflows the signed `Integer` range — either the millisecond scaling or the trailing addition — which can occur only for an instant at the extreme edge of the timeline. [[src/builtins/datetime_package.mfb:__datetime_toMillis]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Epoch milliseconds of the current instant:

```
IMPORT datetime

LET ms AS Integer = datetime::toMillis(datetime::now())
```

Round-trip an instant through its millisecond count:

```
IMPORT datetime

LET at AS Instant = datetime::now()
LET ms AS Integer = datetime::toMillis(at)
LET back AS Instant = datetime::fromMillis(ms)
```

## See also

- `mfb man datetime fromMillis`
- `mfb man datetime toNanos`
- `mfb man datetime instant`
- `mfb man datetime now`
