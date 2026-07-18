# inZone

Project an absolute `Instant` into a `Zone` to produce a civil `DateTime`.

## Synopsis

```
datetime::inZone(at AS Instant, zone AS Zone) AS DateTime
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

`datetime::inZone` is the primary "to civil time" call: it projects the absolute
instant `at` through `zone`, yielding the calendar date and wall-clock time that
an observer in that zone reads at that moment.

It first resolves the effective UTC offset for `zone` at the instant `at` — the
same quantity `datetime::offsetAt` returns: zero for a UTC zone
(`ZoneKind::Utc`), the stored constant for a fixed-offset zone (`ZoneKind::FixedOffset`,
kind `1`, built with `datetime::fixedOffset`), and the DST-correct host offset
for a local zone (`ZoneKind::Local`, kind `2`, built with `datetime::local`).
[[src/builtins/datetime_package.mfb:ZoneKind]] [[src/builtins/datetime_package.mfb:__datetime_offsetAt]] It then adds
that offset, in seconds, to the instant's seconds-since-epoch to obtain a local
second count, floor-divides that into whole days and the second-of-day, converts
the day count to a civil year/month/day with the proleptic Gregorian calendar,
and decomposes the second-of-day into hour, minute, and second.
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

The returned `DateTime` carries four things: the civil date, the civil time,
`zone` itself, and the resolved offset. Because the offset is pinned onto the
result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second `nanos`
field is preserved verbatim into the time's `nanos` field; only the `seconds`
field participates in the offset and date/time computation, so an instant before
the Unix epoch (negative `seconds`) projects correctly.
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

`datetime::toUtc` and `datetime::toLocal` are shorthands for calling `inZone`
with the UTC zone and the host local zone, respectively. `inZone` is pure for UTC
and fixed-offset zones; for a local zone it reads the host's time-zone
configuration through the `datetime::localOffset` OS intrinsic to resolve the
offset. [[src/builtins/datetime_package.mfb:__datetime_toUtc]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `at` | `Instant` | The absolute point on the UTC timeline to project. Its `seconds` field (seconds since the Unix epoch, may be negative) drives the offset resolution and the civil date/time computation; its `nanos` field is copied unchanged into the result's time. [[src/builtins/datetime.rs:IN_ZONE]] |
| `zone` | `Zone` | The zone to project into. Its kind selects how the offset is resolved: a UTC zone contributes a zero offset, a fixed-offset zone (`datetime::fixedOffset`) contributes its single constant offset, and a local zone (`datetime::local`) contributes the host's DST-correct offset for the instant `at`. |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | A `DateTime` holding the civil date and wall-clock time observed in `zone` at the instant `at`, together with `zone` and the resolved UTC offset in seconds. The `nanos` of the time equal the `nanos` of `at`, and the result resolves back to `at` via `datetime::resolve`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Adding the resolved offset to the instant's seconds-since-epoch (`at.seconds + off`) produces a value outside the signed `Integer` range, which can occur only for an instant at the extreme edge of the timeline. [[src/builtins/datetime_package.mfb:__datetime_inZone]] |

## Examples

Project the current instant into UTC:

```
IMPORT datetime

LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
```

Project an instant into a fixed +05:30 zone:

```
IMPORT datetime

LET z AS Zone = datetime::fixedOffset(5, 30)
LET dt AS DateTime = datetime::inZone(datetime::now(), z)
```

Project into the host's local zone, with DST applied for that instant:

```
IMPORT datetime

LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::local())
```

## See also

- `mfb man datetime offsetAt`
- `mfb man datetime resolve`
- `mfb man datetime civil`
- `mfb man datetime withZone`
- `mfb man datetime toUtc`
- `mfb man datetime toLocal`
