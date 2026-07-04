# toIso

Render a `DateTime` as an RFC 3339 / ISO 8601 timestamp.

## Synopsis

```
datetime::toIso(dt AS DateTime) AS String
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

`datetime::toIso` renders `dt` as an RFC 3339 (ISO 8601 profile) timestamp with
fixed millisecond precision and an explicit UTC offset. The result is a freshly
built `String` of the shape `yyyy-MM-ddTHH:mm:ss.fffZ`, for example
`2026-06-25T14:30:00.000+05:30`, where the literal `T` separates the date from
the time and the trailing field is the offset carried by `dt`: the single letter
`Z` when the offset is zero, otherwise a signed `+HH:MM` or `-HH:MM`. The
fractional-second field is always three digits (milliseconds), zero-padded, even
when `dt` has no sub-second value. [[src/builtins/datetime_package.mfb:__datetime_toIso]]

`toIso` is the convenience form of `datetime::format` invoked with the fixed
pattern `yyyy-MM-dd'T'HH:mm:ss.fffZ`. It reads only the date fields, time
fields, and resolved offset of `dt`; it does not consult `dt`'s zone name, apply
any zone conversion, or shift the moment. The `nanos` of `dt` are truncated to
milliseconds for the `fff` field. `dt` is read only and is not modified. The
output is round-trippable: `datetime::parseIso` parses a string produced by
`toIso` back into an equivalent `DateTime`. [[src/builtins/datetime_package.mfb:__datetime_toIso]]

Because the pattern is fixed and always valid, `toIso` emits a result for every
`DateTime` and is pure: it reads no host state and has no side effects.
[[src/builtins/datetime_package.mfb:__datetime_format]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The moment to render. Its date fields (`year`, `month`, `day`), time fields (`hour`, `minute`, `second`, `nanos`), and resolved UTC `offset` supply the output values; the `nanos` are truncated to milliseconds for the `fff` field, and the offset selects `Z` or a signed `+/-HH:MM` suffix. [[src/builtins/datetime.rs:TO_ISO]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A freshly built RFC 3339 / ISO 8601 timestamp of the form `yyyy-MM-ddTHH:mm:ss.fffZ` with millisecond precision and an explicit offset (`Z` for a zero offset, otherwise `+/-HH:MM`). [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Render the current instant in UTC, yielding a `...Z` suffix:

```
IMPORT datetime

LET dt AS DateTime = datetime::toUtc(datetime::now())
LET text AS String = datetime::toIso(dt)
```

Render a fixed-offset moment, yielding a signed offset suffix:

```
IMPORT datetime

LET z AS Zone = datetime::fixedOffset(5, 30)
LET dt AS DateTime = datetime::parse("2026-06-25 14:30:00", "yyyy-MM-dd HH:mm:ss", z)
LET text AS String = datetime::toIso(dt)
```

Round-trip a timestamp through `toIso` and `parseIso`:

```
IMPORT datetime

LET dt AS DateTime = datetime::toUtc(datetime::now())
LET back AS DateTime = datetime::parseIso(datetime::toIso(dt))
```

## See also

- `mfb man datetime parseIso`
- `mfb man datetime format`
- `mfb man datetime parse`
