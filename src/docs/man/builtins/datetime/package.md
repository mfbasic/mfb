# datetime

Instants, civil dates and times, durations, zones, formatting, and parsing

## Synopsis

```
IMPORT datetime
datetime::now()
datetime::inZone(at, zone)
datetime::format(dt, pattern)
datetime::parse(value, pattern, zone)
datetime::toIso(dt)
```

## Description

The `datetime` package models time around a single source of truth: an `Instant`,
an absolute point on the UTC timeline (Unix epoch, leap-second-free) carrying
whole seconds and a nanosecond field in the range `0 .. 999_999_999`. Everything
civil — `Date`, `Time`, and `DateTime` — is a projection of an instant through a
`Zone`, and every projection records the resolved UTC offset, so a `DateTime`
always knows its offset and round-trips back to its `Instant` without
re-consulting the zone. `datetime` is a built-in package: `IMPORT datetime` needs
no manifest dependency. [[src/builtins/datetime.rs:is_datetime_call]]

All public types are flat, copyable value records and enums — `Instant`,
`Duration`, `Date`, `Time`, `Zone`, `DateTime`, and the enums `ZoneKind`,
`Weekday`, and `Month`. There are no resources and no hidden global state, and the
types are referenced bare (`Instant`, `Date`, …), not package-qualified. Calendar
arithmetic is pure integer math (Howard Hinnant's civil ↔ epoch-day conversions)
and produces identical results on every target. Only three operations touch the
host: the wall clock (`now`), a monotonic counter (`monotonic`), and the local
zone's DST-correct offset (`local`). [[src/builtins/datetime.rs:is_builtin_type]]

Zones come in three kinds. `datetime::utc()` is fixed at offset 0;
`datetime::fixedOffset(...)` builds a constant offset rendered as `+HH:MM`; and
`datetime::local()` resolves the host's zone per-instant, so it is DST-correct at
the moment it projects. Named IANA zones are not supported in this version.
`Instant.seconds` spans the full 64-bit `Integer`, so civil dates reach far beyond
any practical need; `datetime::now()` is additionally bounded by its intrinsic
(nanoseconds since the epoch), valid through year 2262. There are no leap seconds:
every day is 86400 seconds, the POSIX convention. [[src/builtins/datetime_package.mfb:__datetime_fixedOffset1]]

Projection is the primary "to civil" operation: `inZone` maps an instant into a
zone, `toUtc` and `toLocal` are shorthands, and `resolve` maps a civil `DateTime`
back to its `Instant`. Arithmetic operates on instants and durations (`add`,
`subtract`, `between`, `plus`, `minus`, `negate`), on calendar days (`addDays`,
DST-aware) and months (`addMonths`, clamping day-of-month). Formatting and parsing
share a pattern mini-language: a pattern is literal text with token runs, where a
run of the same letter is one token whose length selects width or style, and
literal letters are wrapped in single quotes. `format` renders a `DateTime`,
`parse` reads one back, and `toIso`/`parseIso` handle RFC 3339 / ISO 8601 with a
required offset. [[src/builtins/datetime_package.mfb:__datetime_format]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | raised by `date`, `time`, `instant`, `duration`, and `fixedOffset` when a component field (month, day, hour, minute, second, nanos, or offset magnitude) is outside its valid range [[src/builtins/datetime_package.mfb:__datetime_date]] |
| `77050003` | `ErrInvalidFormat` | raised by `format` on an unknown pattern token, and by `parse` and `parseIso` when the input does not match the pattern, a required field or offset is missing, or a month name is unrecognized [[src/builtins/datetime_package.mfb:__datetime_format]] |
| `77050010` | `ErrOverflow` | raised by `add`, `subtract`, `between`, `addDays`, `addMonths`, `plus`, `minus`, `negate`, `toMillis`, `toNanos`, and other checked arithmetic when a value falls outside the `Integer` range [[src/builtins/datetime_package.mfb:__datetime_toMillis]] |
