# civil

Build a zoned `DateTime` from a civil `Date`, `Time`, and `Zone`.

## Synopsis

```
datetime::civil(date AS Date, time AS Time, zone AS Zone) AS DateTime
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

`datetime::civil` builds a `DateTime` by reading a calendar `date` and a
wall-clock `time` as a local time in `zone`, resolving the UTC offset that
applies to that local moment, and returning the canonical projection of the
resulting `Instant` back through `zone`. Because the result is the projection of
a concrete `Instant`, it round-trips: `datetime::resolve` on the returned
`DateTime` recovers the same `Instant`, and that `Instant` projected through
`zone` with `datetime::inZone` reproduces the same `DateTime` fields.
[[src/builtins/datetime_package.mfb:__datetime_civil]]

The `year`, `month`, and `day` of `date` and the `hour`, `minute`, and `second`
of `time` are combined into a single second count (`daysFromCivil * 86400 +
hour * 3600 + minute * 60 + second`) that names the wall-clock moment, treated
as a civil (zone-local) time. The offset for that moment is then resolved from
`zone`. For a zone with a fixed offset (built by `datetime::utc` or
`datetime::fixedOffset`) the offset is constant; for the host's local zone
(`datetime::local`) it is resolved from the platform's zone table at that
instant, so the result is daylight-saving correct.
[[src/builtins/datetime_package.mfb:__datetime_civil]]

When the named local time does not exist or is not unique because of a
daylight-saving transition, `civil` resolves it deterministically. It probes the
zone's offset one day before and one day after the named local time to bracket
any single nearby transition. If both probes agree, that offset is used
directly. If they differ, a spring-forward gap (the named local time is skipped)
shifts forward onto the post-transition offset, and a fall-back overlap (the
named local time occurs twice) takes the earlier, pre-transition offset.
[[src/builtins/datetime_package.mfb:__datetime_resolveLocal]]

The sub-second `nanos` of `time` are carried through unchanged into the
resulting `Instant` and `DateTime`; only the whole-second civil fields
participate in offset resolution. `civil` is pure: beyond what `zone` itself
resolves it reads no host state and has no side effects.
[[src/builtins/datetime_package.mfb:__datetime_civil]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `date` | `Date` | The calendar date (`year`, `month`, `day`) of the wall-clock moment, interpreted as a local date in `zone`. [[src/builtins/datetime.rs:CIVIL]] |
| `time` | `Time` | The wall-clock time of day (`hour`, `minute`, `second`, `nanos`) of the moment, interpreted as a local time in `zone`. The `nanos` field is preserved verbatim in the result. [[src/builtins/datetime_package.mfb:__datetime_civil]] |
| `zone` | `Zone` | The zone in which `date` and `time` are read as local civil fields and against which the UTC offset is resolved. May be a fixed-offset zone (`datetime::utc`, `datetime::fixedOffset`) or the host's local zone (`datetime::local`). [[src/builtins/datetime_package.mfb:__datetime_offsetAt]] |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | The canonical `DateTime` naming the same moment as `date` and `time` in `zone`, carrying the resolved UTC offset. Round-trips through `datetime::resolve` and `datetime::inZone`. For a spring-forward gap the fields reflect the forward-shifted instant; for a fall-back overlap they reflect the earlier of the two candidate offsets. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The civil-to-seconds arithmetic (`daysFromCivil(...) * 86400 + ...`) or the offset resolution produces a value outside the signed `Integer` range, which can occur only for a `date`/`time` at the extreme edge of the representable timeline. [[src/builtins/datetime_package.mfb:__datetime_civil]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Combine a date and time into a `DateTime` in the local zone:

```
IMPORT datetime

LET d AS Date = datetime::date(2026, 6, 26)
LET tm AS Time = datetime::time(9, 30)
LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
```

Build a `DateTime` in UTC and recover its `Instant`:

```
IMPORT datetime

LET d AS Date = datetime::date(2026, 1, 1)
LET tm AS Time = datetime::time(0, 0)
LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
LET at AS Instant = datetime::resolve(dt)
```

## See also

- `mfb man datetime inZone`
- `mfb man datetime resolve`
- `mfb man datetime date`
- `mfb man datetime time`
- `mfb man datetime local`
