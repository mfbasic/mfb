# local

The `Zone` representing the host's local time.

## Synopsis

```
datetime::local() AS Zone
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

`datetime::local` returns the `Zone` that represents the host's local time. The
returned `Zone` carries a zone kind of `ZoneKind::Local` (the third `ZoneKind`
variant, tag `2`), marking it as the platform-resolved local zone rather than the
canonical UTC zone built by `datetime::utc` (kind `ZoneKind::Utc`, tag `0`) or an
arbitrary fixed offset built by `datetime::fixedOffset` (kind
`ZoneKind::FixedOffset`, tag `1`).
[[src/builtins/datetime_package.mfb:__datetime_local]] [[src/builtins/datetime_package.mfb:ZoneKind]]

Unlike `datetime::utc` and `datetime::fixedOffset`, whose offsets are baked into
the `Zone` at construction, the local zone holds no fixed offset of its own. The
`Zone` returned here stores a placeholder offset of zero seconds and the label
`"Local"`; the true offset is resolved per-instant from the platform's zone
table when the zone is applied to a particular moment. Projecting an `Instant`
through this zone with `datetime::inZone` consults that table for the instant
being projected, so the result is DST-correct: the same local zone yields one
offset for a summer instant and another for a winter instant when the host
observes daylight saving time. `datetime::toLocal` is the dedicated shorthand
for projecting an `Instant` through this zone.

Because the offset is resolved from host configuration, the civil fields a given
`Instant` projects to depend on the machine: two hosts in different configured
time zones project the same `Instant` to different `DateTime` fields.

`datetime::local` takes no arguments. The call itself is pure and constant: it
always returns the same placeholder `Zone`, reads no host state, and has no side
effects. The dependence on the host's configured zone enters only later, when
the zone is resolved against an instant during projection.
[[src/builtins/datetime.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `datetime::local` takes no arguments. [[src/builtins/datetime.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Zone` | The host's local zone: a `Zone` with a zone kind of `ZoneKind::Local` (tag `2`), a placeholder offset of zero seconds, and the label `"Local"`. The same value is returned on every call; its effective offset is determined only when the zone is projected against a specific `Instant`. [[src/builtins/datetime.rs:call_return_type_name]] [[src/builtins/datetime_package.mfb:__datetime_local]] |

## Errors

No errors.

## Examples

Obtain the local zone:

```
IMPORT datetime

LET z AS Zone = datetime::local()
```

Project the current instant into the local zone to read its civil fields:

```
IMPORT datetime

LET t AS Instant = datetime::now()
LET here AS DateTime = datetime::inZone(t, datetime::local())
```

Combine a date and time into a `DateTime` in the local zone:

```
IMPORT datetime

LET d AS Date = datetime::date(2026, 6, 26)
LET tm AS Time = datetime::time(9, 30)
LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
```

## See also

- `mfb man datetime utc`
- `mfb man datetime fixedOffset`
- `mfb man datetime inZone`
- `mfb man datetime toLocal`
- `mfb man datetime offsetAt`
