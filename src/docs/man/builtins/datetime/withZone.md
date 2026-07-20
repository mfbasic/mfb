# withZone

Re-project a `DateTime` into a different `Zone`, preserving the absolute instant.

## Synopsis

```
datetime::withZone(dt AS DateTime, zone AS Zone) AS DateTime
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

`datetime::withZone` returns the civil `DateTime` that an observer in `zone`
reads at the very same absolute moment named by `dt`. The underlying point on the
UTC timeline is unchanged; only the wall-clock fields, the carried `zone`, and the
resolved UTC offset are re-derived for the new zone.

The function is exactly the composition of `datetime::resolve` and
`datetime::inZone`: it collapses `dt` back to an `Instant` with `datetime::resolve`
and then projects that `Instant` into `zone` with `datetime::inZone`.
[[src/builtins/datetime_package.mfb:__datetime_withZone]]

The `resolve` step reads the offset already pinned on `dt` to reach the UTC
timeline without any zone lookup (`daysFromCivil(...) * 86400 + hour * 3600 +
minute * 60 + second - dt.offset`). The `inZone` step then resolves the effective
offset for `zone` at that instant — zero for a UTC zone (`ZoneKind::Utc`), the
stored constant for a fixed-offset zone (`ZoneKind::FixedOffset`, built with
`datetime::fixedOffset`), and the DST-correct host offset for a local zone
(`ZoneKind::Local`, built with `datetime::local`) — adds it to the instant's
seconds, floor-divides into whole days and second-of-day, and splits the result
into civil year/month/day and hour/minute/second with the proleptic Gregorian
calendar. [[src/builtins/datetime_package.mfb:ZoneKind]]
[[src/builtins/datetime_package.mfb:__datetime_resolve]]
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

The returned `DateTime` carries the new civil date and time, `zone` itself, and
the offset resolved for `zone`. The sub-second `nanos` field is carried through
both steps verbatim, so it equals `dt.time.nanos`. Because the instant is
preserved, `datetime::resolve` on the result returns the same `Instant` as
`datetime::resolve` on `dt`: `withZone` is an identity on the absolute moment and
changes only its civil presentation. It is pure for UTC and fixed-offset zones;
for a local zone it reads the host's time-zone configuration through the
`datetime::localOffset` OS intrinsic to resolve the offset.
[[src/builtins/datetime_package.mfb:__datetime_inZone]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The source civil date-time. Its `date`, `time`, and pinned `offset` are used to recover the absolute `Instant` it names; its `zone` field is not consulted for the recovery. The `nanos` of `dt.time` are preserved into the result. [[src/builtins/datetime.rs:WITH_ZONE]] |
| `zone` | `Zone` | The zone to re-project into. Its kind selects how the new offset is resolved: a UTC zone always contributes a zero offset, a fixed-offset zone (`datetime::fixedOffset`) contributes its single constant offset, and a local zone (`datetime::local`) contributes the host's DST-correct offset for the recovered instant. [[src/builtins/datetime_package.mfb:__datetime_inZone]] |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | A `DateTime` holding the civil date and wall-clock time observed in `zone` at the same instant `dt` names, together with `zone` and the offset resolved for it. Its `nanos` equal `dt.time.nanos`, and it resolves back to the same `Instant` as `dt` via `datetime::resolve`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The civil-to-seconds arithmetic in the `resolve` step, the offset subtraction there, or the offset addition (`at.seconds + off`) in the projection step produces a value outside the signed `Integer` range, which can occur only for a `DateTime` at the extreme edge of the representable timeline. [[src/builtins/datetime_package.mfb:__datetime_resolve]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Re-project a UTC `DateTime` into a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET shifted AS DateTime = datetime::withZone(dt, z)
END SUB
```

Convert a `DateTime` to the host's local zone without changing the instant:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET local AS DateTime = datetime::withZone(dt, datetime::local())
END SUB
```

## See also

- `mfb man datetime inZone`
- `mfb man datetime resolve`
- `mfb man datetime civil`
- `mfb man datetime toUtc`
- `mfb man datetime toLocal`
- `mfb man datetime offsetAt`
