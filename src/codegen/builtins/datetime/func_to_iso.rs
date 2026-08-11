//! `datetime::toIso` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/toIso.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Render a `DateTime` as an RFC 3339 / ISO 8601 timestamp."#;
const DESC: &str = r#"`datetime::toIso` renders `dt` as an RFC 3339 (ISO 8601 profile) timestamp with
fixed millisecond precision and an explicit UTC offset. The result is a freshly
built `String` of the shape `yyyy-MM-ddTHH:mm:ss.fffZ`, for example
`2026-06-25T14:30:00.000+05:30`, where the literal `T` separates the date from
the time and the trailing field is the offset carried by `dt`: the single letter
`Z` when the offset is zero, otherwise a signed `+HH:MM` or `-HH:MM`. The
fractional-second field is always three digits (milliseconds), zero-padded, even
when `dt` has no sub-second value.

`toIso` is the convenience form of `datetime::format` invoked with the fixed
pattern `yyyy-MM-dd'T'HH:mm:ss.fffZ`. It reads only the date fields, time
fields, and resolved offset of `dt`; it does not consult `dt`'s zone name, apply
any zone conversion, or shift the moment. The `nanos` of `dt` are truncated to
milliseconds for the `fff` field. `dt` is read only and is not modified. The
output is round-trippable: `datetime::parseIso` parses a string produced by
`toIso` back into an equivalent `DateTime`.

Because the pattern is fixed and always valid, `toIso` emits a result for every
`DateTime` and is pure: it reads no host state and has no side effects."#;
const EX: &str = r#"Render the current instant in UTC, yielding a `...Z` suffix:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET text AS String = datetime::toIso(dt)
END SUB
```

Render a fixed-offset moment, yielding a signed offset suffix:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET dt AS DateTime = datetime::parse("2026-06-25 14:30:00", "yyyy-MM-dd HH:mm:ss", z)
  LET text AS String = datetime::toIso(dt)
END SUB
```

Round-trip a timestamp through `toIso` and `parseIso`:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET back AS DateTime = datetime::parseIso(datetime::toIso(dt))
END SUB
```"#;

pub(crate) const TO_ISO: BuiltinFunction = BuiltinFunction::custom(
    "datetime.toIso",
    "toIso",
    INTRO,
    DESC,
    &[],
    &[super::ov(&[super::req("dt", "DateTime")], "String")],
)
.with_example(EX);
