# fromMillis

Build the `Instant` at a given epoch-millisecond count.

## Synopsis

```
datetime::fromMillis(millis AS Integer) AS Instant
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

`datetime::fromMillis` builds an `Instant` on the UTC timeline (Unix epoch,
leap-second-free) from a single count of whole milliseconds measured from
`1970-01-01T00:00:00Z`. A `millis` of `0` yields the epoch itself, positive
values select instants after the epoch, and negative values select instants
before it. [[src/builtins/datetime_package.mfb:__datetime_fromMillis]]

The count is split into a whole-second `seconds` field and a sub-second `nanos`
field by *floor* division, so the `nanos` remainder is always non-negative. The
implementation first computes the toward-zero quotient `millis / 1000` and
remainder `millis MOD 1000`; when that remainder is negative it adds `1000` to
the remainder and subtracts `1` from the quotient, borrowing one second. The
`seconds` field is therefore the mathematical floor of `millis / 1000` and the
`nanos` field is the borrowed, non-negative millisecond remainder scaled to
nanoseconds (`remainder * 1000000`), always in `0..999000000`. A `millis` of
`-1` produces `seconds` `-1` and `nanos` `999000000`, the instant one
millisecond before the epoch. Because the input carries only millisecond
resolution, the `nanos` field is always a whole number of milliseconds — its
microsecond and nanosecond digits are zero.
[[src/builtins/datetime_package.mfb:__datetime_fromMillis]]

The arithmetic cannot overflow: dividing by `1000` only reduces the magnitude of
the `seconds` field, and the scaled remainder never exceeds `999000000`, so the
result is always representable. `datetime::fromMillis` is pure: it reads no host
state and the same `millis` always yields the same `Instant`.

`datetime::fromMillis` is the inverse of `datetime::toMillis` to
whole-millisecond precision. Because the input has no sub-millisecond component,
round-tripping an arbitrary `Instant` through `datetime::toMillis` and back loses
its microsecond and nanosecond digits; for full nanosecond precision use
`datetime::toNanos` together with `datetime::instant`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `millis` | `Integer` | The number of whole milliseconds from the Unix epoch to the desired instant: `0` for the epoch, positive for instants after it, negative for instants before it. Any `Integer` value is accepted. [[src/builtins/datetime.rs:FROM_MILLIS]] |

## Return value

| Type | Description |
| --- | --- |
| `Instant` | The `Instant` `millis` milliseconds from the Unix epoch. Its `seconds` field holds the floor of `millis / 1000` (negative for instants before the epoch) and its `nanos` field holds the millisecond remainder scaled to nanoseconds, always in `0..999000000`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Build an `Instant` from an epoch-millisecond timestamp:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::fromMillis(1_700_000_000_000)
END SUB
```

Select the instant one millisecond before the epoch:

```
IMPORT datetime

SUB main()
  LET before AS Instant = datetime::fromMillis(-1)
END SUB
```

Round-trip an instant through its millisecond count:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET ms AS Integer = datetime::toMillis(at)
  LET back AS Instant = datetime::fromMillis(ms)
END SUB
```

## See also

- `mfb man datetime toMillis`
- `mfb man datetime toNanos`
- `mfb man datetime instant`
- `mfb man datetime now`
