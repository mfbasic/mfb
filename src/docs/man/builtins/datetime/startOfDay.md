# startOfDay

Return the civil `DateTime` naming midnight at the start of a `DateTime`'s day, in its own zone.

## Synopsis

```
datetime::startOfDay(dt AS DateTime) AS DateTime
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

`datetime::startOfDay` returns the `DateTime` naming `00:00:00` (midnight) at the
beginning of `dt`'s civil day, in `dt`'s own zone. It keeps `dt`'s calendar date
(year, month, day) and zone, replaces the wall-clock time with a `Time` of
`00:00:00` and zero nanoseconds, and re-resolves the moment through that zone.
[[src/builtins/datetime_package.mfb:__datetime_startOfDay]]

The result is produced exactly as `datetime::civil(dt.date, Time[0, 0, 0, 0],
dt.zone)`: local midnight is interpreted in `dt`'s zone, the applicable UTC offset
is resolved for that moment, and the canonical `DateTime` naming the resulting
`Instant` is returned. Because the offset is re-resolved rather than copied from
`dt`, the result is daylight-saving correct: for the host's local zone the offset
reflects whatever DST rule applies at midnight on that date, which may differ from
the offset that applied at `dt`'s original time of day.
[[src/builtins/datetime_package.mfb:__datetime_civil]]

The day boundary is civil midnight in `dt`'s zone, not UTC midnight, so the
underlying `Instant` generally differs from `dt`'s `Instant` truncated to whole
days. Any sub-second nanoseconds carried by `dt` are dropped: the start of the day
has zero nanos. Like `datetime::civil`, the result round-trips through
`datetime::resolve` and `datetime::inZone`.

`datetime::startOfDay` is pure when `dt`'s zone is a fixed-offset zone
(`datetime::utc`, `datetime::fixedOffset`). When `dt`'s zone is the host's local
zone (`datetime::local`), the offset is resolved from the platform's zone table,
so the same `dt` can yield a different absolute instant on a host configured for a
different zone or DST rule.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The `DateTime` whose day is taken. Its calendar date and zone are read; its time of day and sub-second nanoseconds are discarded and replaced with `00:00:00.000000000`. The zone may be fixed-offset or the host's local zone. [[src/builtins/datetime.rs:START_OF_DAY]] |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | A `DateTime` on the same calendar date and in the same zone as `dt`, with the wall-clock time set to `00:00:00` and zero nanoseconds, carrying the UTC offset re-resolved for local midnight on that date. The value round-trips through `datetime::resolve` and `datetime::inZone`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The epoch-seconds arithmetic used to resolve local midnight (`daysFromCivil(...) * 86400`, and the surrounding offset probes) produces a value outside the signed `Integer` range for an extreme calendar date. [[src/builtins/datetime_package.mfb:__datetime_civil]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Truncate a `DateTime` to the start of its civil day:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toLocal(datetime::now())
  LET midnight AS DateTime = datetime::startOfDay(dt)
END SUB
```

Start of day in a fixed UTC zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
  LET midnight AS DateTime = datetime::startOfDay(dt)
END SUB
```

## See also

- `mfb man datetime civil`
- `mfb man datetime addDays`
- `mfb man datetime resolve`
- `mfb man datetime inZone`
