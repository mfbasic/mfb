# types

the datetime package record types

## Synopsis

```
datetime::Instant
datetime::Duration
datetime::Date
datetime::Time
datetime::Zone
datetime::DateTime
```

## Package

datetime

## Imports

`IMPORT datetime`. `datetime` is a built-in package, so the import needs no
manifest dependency. [[src/builtins/datetime.rs:is_datetime_call]]

## Description

The `datetime` package models time around a single source of truth: an `Instant`,
an absolute point on the UTC timeline. Everything civil — `Date`, `Time`, and
`DateTime` — is a projection of an instant through a `Zone`, and a `Duration` is a
signed span between instants. All of these types are flat, copyable value records:
they hold no resources and no hidden state, and they are referenced bare
(`Instant`, `Date`, …), not package-qualified. [[src/builtins/datetime.rs:is_builtin_type]]

`Instant` and `Duration` both split time into whole `seconds` plus a `nanos`
field normalized into the range `0 .. 999_999_999`, so that identical wall spans
have identical representations. A `DateTime` composes a `Date`, a `Time`, and the
`Zone` it was projected through, and additionally caches the resolved UTC `offset`
so it round-trips back to its `Instant` without re-consulting the zone. Zones are
distinguished by their `kind` field, which takes the values of the exported
`ZoneKind` enum (`Utc` = 0, `FixedOffset` = 1, `Local` = 2). [[src/builtins/datetime_package.mfb:__datetime_offsetAt]]

Alongside these records the package exports three enums — `ZoneKind`, `Weekday`,
and `Month` — which carry no fields and so are not tabulated here. [[src/builtins/datetime_package.mfb:__datetime_weekday]]

## Types

### datetime::Instant

An absolute point on the UTC timeline (Unix epoch, leap-second-free).

| Field | Type | Description |
| --- | --- | --- |
| `seconds` | `Integer` | Whole seconds since the Unix epoch (1970-01-01T00:00:00Z); may be negative for instants before the epoch, and spans the full 64-bit `Integer` range. |
| `nanos` | `Integer` | Sub-second nanoseconds, normalized into `0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_normInstant]] |

### datetime::Duration

A signed span of time between two instants.

| Field | Type | Description |
| --- | --- | --- |
| `seconds` | `Integer` | Whole seconds of the span; negative for a backwards span. |
| `nanos` | `Integer` | Sub-second nanoseconds, normalized into `0 .. 999_999_999` (a negative span borrows one second so `nanos` stays non-negative). [[src/builtins/datetime_package.mfb:__datetime_normDuration]] |

### datetime::Date

A civil calendar date (proleptic Gregorian), with no time or zone.

| Field | Type | Description |
| --- | --- | --- |
| `year` | `Integer` | Proleptic Gregorian year (e.g. `2026`); may be negative for years before 1 CE. |
| `month` | `Integer` | Month of the year, `1 .. 12` (January = 1). |
| `day` | `Integer` | Day of the month, `1 .. daysInMonth(year, month)`. [[src/builtins/datetime_package.mfb:__datetime_date]] |

### datetime::Time

A civil wall-clock time of day, with no date or zone.

| Field | Type | Description |
| --- | --- | --- |
| `hour` | `Integer` | Hour of the day, `0 .. 23`. |
| `minute` | `Integer` | Minute of the hour, `0 .. 59`. |
| `second` | `Integer` | Second of the minute, `0 .. 59` (no leap seconds). |
| `nanos` | `Integer` | Sub-second nanoseconds, `0 .. 999_999_999`. [[src/builtins/datetime_package.mfb:__datetime_time]] |

### datetime::Zone

A time zone: either UTC, a fixed offset, or the host's local zone.

| Field | Type | Description |
| --- | --- | --- |
| `offsetSeconds` | `Integer` | UTC offset in seconds for `Utc` and `FixedOffset` zones (magnitude under 24h); `0` and unused for a `Local` zone, whose offset is resolved per-instant. |
| `kind` | `Integer` | Which kind of zone this is, a `ZoneKind` value: `Utc` = 0, `FixedOffset` = 1, `Local` = 2. |
| `label` | `String` | Display label, e.g. `"UTC"`, `"Local"`, or a rendered offset such as `"+05:30"`. [[src/builtins/datetime_package.mfb:__datetime_offsetLabel]] |

### datetime::DateTime

An instant projected into a zone: civil date and time plus the zone and its resolved offset.

| Field | Type | Description |
| --- | --- | --- |
| `date` | `Date` | The civil date in the projecting zone. |
| `time` | `Time` | The civil wall-clock time in the projecting zone. |
| `zone` | `Zone` | The zone this civil value was projected through. |
| `offset` | `Integer` | The resolved UTC offset in seconds at this instant, so the value round-trips back to its `Instant` without re-consulting the zone. [[src/builtins/datetime_package.mfb:__datetime_inZone]] |

## See also

- `mfb man datetime`
- `mfb man datetime instant`
- `mfb man datetime inZone`
- `mfb man datetime resolve`
