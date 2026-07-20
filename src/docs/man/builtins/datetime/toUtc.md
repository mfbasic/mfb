# toUtc

Project an absolute `Instant` into UTC to produce a civil `DateTime`.

## Synopsis

```
datetime::toUtc(at AS Instant) AS DateTime
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

`datetime::toUtc` projects the absolute instant `at` into the UTC zone, yielding
the calendar date and wall-clock time that an observer reading UTC sees at that
moment. It is exactly shorthand for `datetime::inZone(at, datetime::utc())`: the
UTC zone contributes a zero offset, so the instant's seconds-since-epoch are
split directly — floor-divided into whole days and the second-of-day — into a
civil year/month/day (proleptic Gregorian calendar) and an
hour/minute/second-of-day, with no offset adjustment.
[[src/builtins/datetime_package.mfb:__datetime_toUtc]]

The returned `DateTime` carries four things: the civil date, the civil time, the
UTC zone, and a resolved offset of zero. Because the zero offset is pinned onto
the result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second
`nanos` field is preserved verbatim into the time's `nanos` field; only the
`seconds` field participates in the date and time computation, so an instant
before the Unix epoch (negative `seconds`) projects correctly.
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

Unlike `datetime::toLocal`, `datetime::toUtc` is pure: it reads no host
time-zone configuration and produces the same result on every platform. Because
the resolved offset is always zero, adding it to the instant's seconds cannot
overflow the `Integer` range, so this call raises no error of its own.
[[src/builtins/datetime_package.mfb:__datetime_utc]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `at` | `Instant` | The absolute point on the UTC timeline to project. Its `seconds` field (seconds since the Unix epoch, may be negative) drives the civil date/time computation; its `nanos` field is copied unchanged into the result's time. [[src/builtins/datetime.rs:TO_UTC]] |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | A `DateTime` holding the civil date and wall-clock time observed in UTC at the instant `at`, together with the UTC zone and a resolved offset of zero seconds. The `nanos` of the time equal the `nanos` of `at`, and the result resolves back to `at` via `datetime::resolve`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Project the current instant into UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
END SUB
```

Round-trip an instant through UTC and back:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET dt AS DateTime = datetime::toUtc(at)
  LET back AS Instant = datetime::resolve(dt)
END SUB
```

## See also

- `mfb man datetime inZone`
- `mfb man datetime toLocal`
- `mfb man datetime utc`
- `mfb man datetime resolve`
