# toLocal

Project an absolute `Instant` into the host's local zone to produce a civil `DateTime`.

## Synopsis

```
datetime::toLocal(at AS Instant) AS DateTime
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

`datetime::toLocal` projects the absolute instant `at` into the host's local
time zone, yielding the calendar date and wall-clock time that an observer
reading the local clock sees at that moment. It is exactly shorthand for
`datetime::inZone(at, datetime::local())`: it resolves the host's effective UTC
offset for the instant `at` (see `datetime::offsetAt`), with daylight-saving
time applied as it stood at that instant, adds that offset in seconds to the
instant's seconds-since-epoch to obtain a local second count, floor-divides that
into whole days and the second-of-day, converts the day count to a civil
year/month/day with the proleptic Gregorian calendar, and decomposes the
second-of-day into hour, minute, and second.
[[src/builtins/datetime_package.mfb:__datetime_toLocal]]
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

The returned `DateTime` carries four things: the civil date, the civil time, the
local zone, and the resolved offset. Because the resolved offset is pinned onto
the result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second
`nanos` field is preserved verbatim into the time's `nanos` field; only the
`seconds` field participates in the offset and date/time computation, so an
instant before the Unix epoch (negative `seconds`) projects correctly.
[[src/builtins/datetime_package.mfb:__datetime_offsetAt]]

Unlike `datetime::toUtc`, `datetime::toLocal` is not pure: it reads the host's
time-zone configuration to resolve the offset, so the same instant can produce a
different civil `DateTime` on a host configured for a different zone or under a
different DST rule.
[[src/builtins/datetime_package.mfb:__datetime_local]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `at` | `Instant` | The absolute point on the UTC timeline to project. Its `seconds` field (seconds since the Unix epoch, may be negative) drives the host offset resolution and the civil date/time computation; its `nanos` field is copied unchanged into the result's time. [[src/builtins/datetime.rs:TO_LOCAL]] |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | A `DateTime` holding the civil date and wall-clock time observed in the host's local zone at the instant `at`, together with the local zone and the resolved UTC offset in seconds (DST-correct for `at`). The `nanos` of the time equal the `nanos` of `at`, and the result resolves back to `at` via `datetime::resolve`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Adding the resolved local offset to the instant's seconds-since-epoch produces a value outside the signed `Integer` range, which can occur only for an instant at the extreme edge of the timeline. [[src/builtins/datetime_package.mfb:__datetime_inZone]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Project the current instant into the host's local zone:

```
IMPORT datetime

LET dt AS DateTime = datetime::toLocal(datetime::now())
```

Round-trip an instant through the local zone and back:

```
IMPORT datetime

LET at AS Instant = datetime::now()
LET dt AS DateTime = datetime::toLocal(at)
LET back AS Instant = datetime::resolve(dt)
```

## See also

- `mfb man datetime inZone`
- `mfb man datetime toUtc`
- `mfb man datetime local`
- `mfb man datetime resolve`
