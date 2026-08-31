//! `datetime::toIso` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Render a `datetime::DateTime` as an RFC 3339 / ISO 8601 timestamp."#;
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
`toIso` back into an equivalent `datetime::DateTime`.

Because the pattern is fixed and always valid, `toIso` emits a result for every
`datetime::DateTime` and is pure: it reads no host state and has no side effects."#;
const EX: &str = r#"Render the current instant in UTC, yielding a `...Z` suffix:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::toUtc(datetime::now())
  LET text AS String = datetime::toIso(dt)
END SUB
```

Render a fixed-offset moment, yielding a signed offset suffix:

```
IMPORT datetime

SUB main()
  LET z AS datetime::Zone = datetime::fixedOffset(5, 30)
  LET dt AS datetime::DateTime = datetime::parse("2026-06-25 14:30:00", "yyyy-MM-dd HH:mm:ss", z)
  LET text AS String = datetime::toIso(dt)
END SUB
```

Round-trip a timestamp through `toIso` and `parseIso`:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::toUtc(datetime::now())
  LET back AS datetime::DateTime = datetime::parseIso(datetime::toIso(dt))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_toIso(dt AS DateTime) AS String
  RETURN __datetime_padN(dt.date.year, 4) & "-" & __datetime_pad2(dt.date.month) & "-" & __datetime_pad2(dt.date.day) & "T" & __datetime_pad2(dt.time.hour) & ":" & __datetime_pad2(dt.time.minute) & ":" & __datetime_pad2(dt.time.second) & "." & strings::left(__datetime_padN(dt.time.nanos, 9), 3) & __datetime_isoZone(dt.offset)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "toIso",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("DateTime"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "dt",
                desc: "The date-time to render as ISO-8601.",
                aliases: &[],
                ty: super::ParameterType::named("DateTime"),
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::String,
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_toIso"),
        }],
    });
}
