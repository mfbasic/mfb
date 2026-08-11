//! `datetime::parseIso` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/parseIso.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Parse an RFC 3339 / ISO 8601 timestamp into a `DateTime`."#;
const DESC: &str = r#"`datetime::parseIso` reads an RFC 3339 (ISO 8601 profile) timestamp from `value`
and returns the `DateTime` it names. It is the convenience inverse of
`datetime::toIso`, and a fixed-shape alternative to `datetime::parse`: rather than
taking a pattern, it expects the canonical RFC 3339 layout

```
yyyy-MM-dd<sep>HH:mm:ss[.fraction]<offset>
```

parsing `value` left to right. The components are:

- `yyyy-MM-dd` — four-digit year, two-digit month, two-digit day, each introduced
  by its literal `-` separator
- `<sep>` — the date/time separator: `T`, `t`, or a single space
- `HH:mm:ss` — two-digit hour, minute, and second on a 24-hour clock, separated by
  literal `:` characters
- `.fraction` — optional fractional second: a `.` followed by decimal digits. The
  first nine digits are scaled to nanoseconds (so `.25` becomes `250000000` ns);
  any digits beyond the ninth are consumed but ignored
- `<offset>` — required UTC offset: `Z` or `z` for UTC, otherwise a signed
  `+/-HH:MM` or `+/-HHMM` (the colon between offset hours and minutes is optional)

The numeric readers are greedy up to their stated width but also accept fewer
digits, so a field may be written with or without leading padding as long as the
surrounding separators are present. The offset is mandatory; unlike
`datetime::parse` there is no zone argument and no defaulting to UTC, because a
conforming RFC 3339 timestamp always carries its own offset. The parsed offset is
applied directly, making the result a fixed-offset moment.

Like `datetime::parse`, `parseIso` does not range-check the decoded calendar
fields: an out-of-range component such as month 13 or day 40 is carried into the
resulting `DateTime` rather than rejected. The one validated numeric range is the
offset, whose magnitude must be under 24 hours. `parseIso` is pure: it reads no
host state and has no side effects."#;
const EX: &str = r#"Parse a UTC timestamp:

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
```"#;

pub(crate) const PARSE_ISO: BuiltinFunction = BuiltinFunction::custom(
    "datetime.parseIso",
    "parseIso",
    INTRO,
    DESC,
    &[],
    &[super::ov(&[super::req("value", "String")], "DateTime")],
)
.with_example(EX);
