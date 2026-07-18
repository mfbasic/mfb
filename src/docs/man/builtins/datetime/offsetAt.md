# offsetAt

A `Zone`'s signed UTC offset in seconds at a given `Instant`.

## Synopsis

```
datetime::offsetAt(zone AS Zone, at AS Instant) AS Integer
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

`datetime::offsetAt` returns the signed offset from UTC, in seconds, that
`zone` applies to the absolute instant `at`. A positive result places the
zone's civil fields ahead of UTC (east of the prime meridian), a negative
result places them behind UTC (west), and zero means the zone coincides with
UTC at that instant. This is the exact quantity `datetime::inZone` adds to an
`Instant`'s seconds-since-epoch to produce the civil fields of a `DateTime`, so
`offsetAt` exposes that adjustment on its own.
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

How the offset is determined depends on the zone's kind. For a UTC zone
(`ZoneKind::Utc`) and a fixed-offset zone (`ZoneKind::FixedOffset`, built with
`datetime::fixedOffset`) the function returns the zone's stored constant offset
directly and does not consult `at` — the UTC zone stores zero, and a fixed zone
stores its single configured offset. For a local zone (`ZoneKind::Local`, built
with `datetime::local`, internally zone kind `2`) the offset is resolved
against the host's configured time zone for the specific instant `at`: it reads
the host zone table and is therefore DST-correct, returning the standard-time
offset for instants outside daylight saving and the shifted offset for instants
within it. Two calls with the same local zone but instants on opposite sides of
a DST transition can therefore return different values.
[[src/builtins/datetime_package.mfb:__datetime_offsetAt]] [[src/builtins/datetime_package.mfb:ZoneKind]]

Only the `seconds` field of `at` participates; the sub-second `nanos` field is
ignored. The function reads no host state for UTC and fixed zones (those are
pure); for a local zone it reads the host's time-zone configuration through the
`datetime::localOffset` OS intrinsic. [[src/builtins/datetime_package.mfb:__datetime_offsetAt]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `zone` | `Zone` | The zone whose offset is queried. Its kind (UTC, fixed, or local) selects how the offset is computed. [[src/builtins/datetime.rs:OFFSET_AT]] |
| `at` | `Instant` | The absolute instant at which to evaluate the offset. Its `seconds` field (seconds since the Unix epoch) is the point on the timeline used; the `nanos` field is ignored. For UTC and fixed zones the instant has no effect on the result; for a local zone it selects the standard or daylight-saving offset in force at that moment. |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The signed offset from UTC in seconds in force for `zone` at instant `at`: always `0` for a UTC zone, the stored constant for a fixed zone, and the DST-correct host offset for a local zone. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

A UTC zone always reports a zero offset:

```
IMPORT datetime

LET off AS Integer = datetime::offsetAt(datetime::utc(), datetime::now())
```

A fixed zone reports its constant offset regardless of the instant:

```
IMPORT datetime

LET z AS Zone = datetime::fixedOffset(5, 30)
LET off AS Integer = datetime::offsetAt(z, datetime::now())
```

A local zone's offset is resolved DST-correctly for the given instant:

```
IMPORT datetime

LET nowOff AS Integer = datetime::offsetAt(datetime::local(), datetime::now())
```

## See also

- `mfb man datetime local`
- `mfb man datetime utc`
- `mfb man datetime fixedOffset`
- `mfb man datetime inZone`
