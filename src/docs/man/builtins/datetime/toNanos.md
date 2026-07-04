# toNanos

Return the whole nanoseconds between the Unix epoch and an `Instant`.

## Synopsis

```
datetime::toNanos(at AS Instant) AS Integer
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

`datetime::toNanos` collapses the absolute point `at` into a single `Integer`
count of whole nanoseconds measured from the Unix epoch
(`1970-01-01T00:00:00Z`). Instants before the epoch yield negative counts, the
epoch itself yields `0`, and instants after the epoch yield positive counts.
[[src/builtins/datetime_package.mfb:__datetime_toNanos]]

The result is computed as `at.seconds * 1000000000 + at.nanos`: the
seconds-since-epoch field is scaled to nanoseconds and the sub-second `nanos`
field is added in directly. Because a normalized `Instant` already holds its
`nanos` field at full nanosecond resolution (`0..999999999`), the conversion is
exact and discards nothing — no truncation or rounding occurs in either
direction.

The arithmetic is checked. For an instant near the extreme edge of the timeline
the `at.seconds * 1000000000` scaling can exceed the signed `Integer` range, in
which case the function raises `ErrOverflow` rather than wrapping. The range of
representable instants is therefore narrower than for `datetime::toMillis`, since
each second consumes a billion units rather than a thousand.
`datetime::toNanos` is pure: it reads no host state and depends only on `at`.
[[src/builtins/datetime_package.mfb:__datetime_toMillis]]

Unlike `datetime::toMillis`, `datetime::toNanos` preserves the full sub-second
precision of `at`; use it when nanosecond fidelity matters.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `at` | `Instant` | The absolute point on the UTC timeline to measure. Its `seconds` field (seconds since the Unix epoch, possibly negative) and its `nanos` field (`0..999999999`) together determine the nanosecond count. Both fields contribute exactly. [[src/builtins/datetime.rs:TO_NANOS]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of whole nanoseconds from the Unix epoch to `at`: negative before the epoch, `0` at the epoch, positive after. The value is exact, capturing the complete sub-second precision of `at`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Scaling `at.seconds` to nanoseconds (`at.seconds * 1000000000`) produces a value outside the signed `Integer` range, which can occur only for an instant at the extreme edge of the timeline. [[src/builtins/datetime_package.mfb:__datetime_toNanos]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Epoch nanoseconds of the current instant:

```
IMPORT datetime

LET ns AS Integer = datetime::toNanos(datetime::now())
```

Compare two instants at nanosecond resolution:

```
IMPORT datetime

LET a AS Integer = datetime::toNanos(datetime::now())
LET b AS Integer = datetime::toNanos(datetime::now())
LET elapsed AS Integer = b - a
```

## See also

- `mfb man datetime toMillis`
- `mfb man datetime fromMillis`
- `mfb man datetime instant`
- `mfb man datetime now`
