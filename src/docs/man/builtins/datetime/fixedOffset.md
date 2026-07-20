# fixedOffset

Build a `Zone` with a constant UTC offset.

## Synopsis

```
datetime::fixedOffset(offsetSeconds AS Integer) AS Zone
datetime::fixedOffset(hours AS Integer, mins AS Integer) AS Zone
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

`datetime::fixedOffset` builds a `Zone` whose offset from UTC is a constant
value that does not vary with the instant being projected. Unlike
`datetime::local`, whose offset is resolved against the host's configured time
zone, and unlike `datetime::utc`, the canonical zero-offset zone, a
fixed-offset `Zone` carries a single signed offset that applies to every
`Instant` projected through it. The returned `Zone` has a zone kind of
`ZoneKind::FixedOffset` and a label rendered in the form `+HH:MM` or `-HH:MM`.
[[src/builtins/datetime_package.mfb:__datetime_fixedOffset1]] [[src/builtins/datetime_package.mfb:ZoneKind]]

A `Zone` is the bridge between the absolute UTC timeline (an `Instant`) and the
human-readable civil fields of a `DateTime`. Projecting an `Instant` through a
fixed-offset zone with `datetime::inZone` produces a `DateTime` whose year,
month, day, and time fields are shifted from UTC by exactly the offset this
function encodes: a positive offset places the civil fields ahead of UTC (east
of the prime meridian), a negative offset places them behind UTC (west).

The one-argument form takes the offset directly as a raw signed second count.
The two-argument form takes whole `hours` and a `mins` magnitude in the range
`0 .. 59`; `mins` contributes its magnitude only and inherits the sign of
`hours`. Thus `datetime::fixedOffset(-5, 30)` is `-05:30` (five hours and
thirty minutes behind UTC), and `datetime::fixedOffset(5, 30)` is `+05:30`. The
two-argument form is implemented in terms of the one-argument form by combining
the hours and minutes into a total second count of
`sign(hours) * (abs(hours) * 3600 + mins * 60)`.
[[src/builtins/datetime_package.mfb:__datetime_fixedOffset2]]

In both forms the offset magnitude must be strictly under 24 hours (86400
seconds); an offset of exactly `+/-24h` or more is rejected. The function is
pure: it reads no host state and has no side effects.

## Overloads

**`datetime::fixedOffset(offsetSeconds AS Integer) AS Zone`**

Builds a fixed zone whose offset is `offsetSeconds`, interpreted as a signed
count of seconds east (positive) or west (negative) of UTC.

**`datetime::fixedOffset(hours AS Integer, mins AS Integer) AS Zone`**

Builds a fixed zone from whole `hours` and a `0 .. 59` minute magnitude. The
minutes take the sign of `hours`, so the combined offset is
`sign(hours) * (abs(hours) * 3600 + mins * 60)` seconds.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `offsetSeconds` | `Integer` | The signed offset from UTC in seconds. Positive is east of UTC, negative is west. The magnitude must be strictly less than `86400` (24 hours). [[src/builtins/datetime_package.mfb:__datetime_fixedOffset1]] |
| `hours` | `Integer` | The whole-hour component of the offset, signed. Its sign determines the sign of the whole resulting offset, including the minutes contribution. |
| `mins` | `Integer` | The minute magnitude of the offset, in the range `0 .. 59`. It contributes its magnitude only and inherits the sign of `hours`. [[src/builtins/datetime_package.mfb:__datetime_fixedOffset2]] |

## Return value

| Type | Description |
| --- | --- |
| `Zone` | A `Zone` with a zone kind of `ZoneKind::FixedOffset`, the requested constant offset, and a label of the form `+HH:MM` or `-HH:MM`. For a zero offset the label is `+00:00`. [[src/builtins/datetime.rs:call_return_type_name]] [[src/builtins/datetime_package.mfb:__datetime_offsetLabel]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | The resulting offset magnitude is 24 hours (`86400` seconds) or more, or, in the two-argument form, `mins` is outside the range `0 .. 59`. [[src/builtins/datetime_package.mfb:__datetime_fixedOffset1]] |

## Examples

Build a zone five and a half hours behind UTC:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-5, 30)
END SUB
```

Build the same zone from a raw second count:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-19800)
END SUB
```

Project the current instant into a fixed `+09:00` zone:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET local AS DateTime = datetime::inZone(t, datetime::fixedOffset(9, 0))
END SUB
```

## See also

- `mfb man datetime utc`
- `mfb man datetime local`
- `mfb man datetime inZone`
- `mfb man datetime offsetAt`
