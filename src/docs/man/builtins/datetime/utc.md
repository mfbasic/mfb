# utc

The `Zone` representing Coordinated Universal Time.

## Synopsis

```
datetime::utc() AS Zone
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

`datetime::utc` returns the `Zone` that represents Coordinated Universal Time: a
fixed zone whose offset from UTC is a constant zero seconds and whose label is
the literal string `"UTC"`. The returned `Zone` carries a zone kind of
`ZoneKind::Utc` (the first `ZoneKind` variant, tag `0`), marking it as the
canonical UTC zone rather than an arbitrary fixed offset built with
`datetime::fixedOffset` (kind `ZoneKind::FixedOffset`).
[[src/builtins/datetime_package.mfb:__datetime_utc]] [[src/builtins/datetime_package.mfb:ZoneKind]]

A `Zone` is the bridge between the absolute UTC timeline (an `Instant`) and the
human-readable civil fields of a `DateTime`. Project an `Instant` through this
zone with `datetime::inZone` to obtain a `DateTime` whose year, month, day, and
time fields are expressed in UTC; `datetime::toUtc` is the dedicated shorthand
for exactly that projection. Because the offset is always zero, the civil fields
of a `DateTime` in this zone match the seconds-since-epoch of the originating
`Instant` directly, with no offset adjustment.

`datetime::utc` takes no arguments and always returns the same constant `Zone`.
It is pure: every call yields an identical UTC zone, it reads no host state, and
it has no side effects. Unlike `datetime::local`, whose offset depends on the
host's configured time zone, `datetime::utc` is wholly independent of the
environment. [[src/builtins/datetime.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `datetime::utc` takes no arguments. [[src/builtins/datetime.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Zone` | The UTC zone: a `Zone` with a constant offset of zero seconds, a zone kind of `ZoneKind::Utc` (tag `0`), and the label `"UTC"`. The same value is returned on every call. [[src/builtins/datetime.rs:call_return_type_name]] [[src/builtins/datetime_package.mfb:__datetime_utc]] |

## Errors

No errors.

## Examples

Obtain the UTC zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::utc()
END SUB
```

Project the current instant into UTC to read its civil fields:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET inUtc AS DateTime = datetime::inZone(t, datetime::utc())
END SUB
```

Combine a date and time into a UTC-zoned `DateTime`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
END SUB
```

## See also

- `mfb man datetime local`
- `mfb man datetime fixedOffset`
- `mfb man datetime inZone`
- `mfb man datetime toUtc`
