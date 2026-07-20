# parseIso

Parse an RFC 3339 / ISO 8601 timestamp into a `DateTime`.

## Synopsis

```
datetime::parseIso(value AS String) AS DateTime
```

## Package

datetime

## Imports

```
IMPORT datetime
```

`datetime` is a built-in package, so `IMPORT datetime` needs no manifest
dependency. [[src/builtins/datetime.rs:uses_package]]

## Description

`datetime::parseIso` reads an RFC 3339 (ISO 8601 profile) timestamp from `value`
and returns the `DateTime` it names. It is the convenience inverse of
`datetime::toIso`, and a fixed-shape alternative to `datetime::parse`: rather than
taking a pattern, it expects the canonical RFC 3339 layout

```
yyyy-MM-dd<sep>HH:mm:ss[.fraction]<offset>
```

parsing `value` left to right. The components are:

- `yyyy-MM-dd` — four-digit year, two-digit month, two-digit day, each introduced
  by its literal `-` separator
- `<sep>` — the date/time separator: `T`, `t`, or a single space [[src/builtins/datetime_package.mfb:__datetime_parseIso]]
- `HH:mm:ss` — two-digit hour, minute, and second on a 24-hour clock, separated by
  literal `:` characters
- `.fraction` — optional fractional second: a `.` followed by decimal digits. The
  first nine digits are scaled to nanoseconds (so `.25` becomes `250000000` ns);
  any digits beyond the ninth are consumed but ignored [[src/builtins/datetime_package.mfb:__datetime_parseIso]]
- `<offset>` — required UTC offset: `Z` or `z` for UTC, otherwise a signed
  `+/-HH:MM` or `+/-HHMM` (the colon between offset hours and minutes is optional)

The numeric readers are greedy up to their stated width but also accept fewer
digits, so a field may be written with or without leading padding as long as the
surrounding separators are present. The offset is mandatory; unlike
`datetime::parse` there is no zone argument and no defaulting to UTC, because a
conforming RFC 3339 timestamp always carries its own offset. The parsed offset is
applied directly, making the result a fixed-offset moment. [[src/builtins/datetime_package.mfb:__datetime_parseIso]]

Like `datetime::parse`, `parseIso` does not range-check the decoded calendar
fields: an out-of-range component such as month 13 or day 40 is carried into the
resulting `DateTime` rather than rejected. The one validated numeric range is the
offset, whose magnitude must be under 24 hours. `parseIso` is pure: it reads no
host state and has no side effects. [[src/builtins/datetime_package.mfb:__datetime_fixedOffset1]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The timestamp text. It must follow the RFC 3339 layout above position for position: each literal separator (`-`, `:`, the date/time separator, and the offset introducer) must appear where expected, and each numeric field must supply the digits it requires. The fractional-second part is the only optional element; the offset is required. [[src/builtins/datetime.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `DateTime` | A fixed-offset `DateTime` built from the decoded year, month, day, hour, minute, second, and fractional nanoseconds, carrying the offset named by `value`'s `Z` / `+HH:MM` field. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `value` does not conform to RFC 3339: a required digit is missing, a `-`, `:`, or date/time separator is absent or wrong, or the offset is missing or malformed (neither `Z`/`z` nor a signed `+/-HH:MM` offset). [[src/builtins/datetime_package.mfb:__datetime_parseIso]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050002` | `ErrInvalidArgument` | The parsed offset decodes to a magnitude of 24 hours (86400 seconds) or more, which is out of range for a fixed-offset zone. [[src/builtins/datetime_package.mfb:__datetime_fixedOffset1]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Parse a UTC timestamp:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parseIso("1969-07-20T20:17:00Z")
END SUB
```

Parse a fractional second with a positive offset:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parseIso("2026-06-25T14:30:00.250+05:30")
END SUB
```

A space may stand in for the `T` separator:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parseIso("2026-06-26 09:30:00-08:00")
END SUB
```

Text that is missing its required offset is not valid RFC 3339 and raises
`ErrInvalidFormat`:

```
IMPORT datetime

SUB main()
  LET bad AS DateTime = datetime::parseIso("2026-06-26T09:30:00")
END SUB
```

## See also

- `mfb man datetime toIso`
- `mfb man datetime parse`
- `mfb man datetime format`
- `mfb man datetime civil`
