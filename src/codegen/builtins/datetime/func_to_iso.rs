//! `datetime::toIso` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Render a `datetime::DateTime` as an RFC 3339 / ISO 8601 timestamp."#;
const DESC: &str = r#"`datetime::toIso` renders `dt` as an RFC 3339 (ISO 8601 profile) timestamp with
an explicit UTC offset. The result is a freshly built `String` of the shape
`yyyy-MM-ddTHH:mm:ss.fffZ`, for example `2026-06-25T14:30:00.000+05:30`, where
the literal `T` separates the date from the time and the trailing field is the
offset carried by `dt`: the single letter `Z` when the offset is zero, otherwise
a signed `+HH:MM` or `-HH:MM`. The fractional-second field is zero-padded to a
fixed width, so the output of a given form is always the same length and sorts
correctly as text.

The one-argument form emits **three** fractional digits (milliseconds). The
two-argument form chooses the width: `digits` may be `0`, `3`, `6` or `9`, and
`0` omits the fractional field entirely (RFC 3339 permits its absence). Any
other value raises `ErrInvalidArgument`; the four allowed widths are exactly the
`fff` / `ffffff` / `fffffffff` tokens `datetime::format` and `datetime::parseIso`
already handle, so every form this member emits can be read back.

**Precision, and what round-trips.** A `datetime::DateTime` carries nanoseconds,
so only `datetime::toIso(dt, 9)` is lossless. It is the one form for which
`datetime::parseIso(datetime::toIso(dt, 9))` recovers `dt`'s `nanos` exactly.
Every narrower form *truncates* — `datetime::toIso(dt)` and
`datetime::toIso(dt, 3)` round-trip only to the millisecond, discarding up to
999999 ns, and `datetime::toIso(dt, 0)` discards the whole sub-second value.
Truncation is toward zero: the digits beyond the chosen width are dropped, never
rounded. Use `9` when the value must survive; use the default when a
fixed-width millisecond timestamp is what the consumer expects.

`toIso` is the convenience form of `datetime::format` invoked with a fixed
pattern (`yyyy-MM-dd'T'HH:mm:ss.fffZ` for the default form). It reads only the
date fields, time fields, and resolved offset of `dt`; it does not consult
`dt`'s zone name, apply any zone conversion, or shift the moment. `dt` is read
only and is not modified.

Apart from a `digits` outside the allowed set, `toIso` emits a result for every
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

Round-trip a timestamp through `toIso` and `parseIso`. The default form is exact
only to the millisecond, so ask for nine digits when the `nanos` must survive:

```
IMPORT datetime
IMPORT io

SUB main()
  LET t AS datetime::Time = datetime::time(1, 2, 3, 123456789)
  LET dt AS datetime::DateTime = datetime::civil(datetime::date(2026, 6, 26), t, datetime::utc())
  io::print(datetime::toIso(dt))
  io::print(datetime::toIso(dt, 9))
  io::print(toString(datetime::parseIso(datetime::toIso(dt)).time.nanos))
  io::print(toString(datetime::parseIso(datetime::toIso(dt, 9)).time.nanos))
END SUB
```

prints:

```
2026-06-26T01:02:03.123Z
2026-06-26T01:02:03.123456789Z
123000000
123456789
```

Drop the fractional field entirely with `digits = 0`:

```
IMPORT datetime
IMPORT io

SUB main()
  LET t AS datetime::Time = datetime::time(1, 2, 3, 123456789)
  LET dt AS datetime::DateTime = datetime::civil(datetime::date(2026, 6, 26), t, datetime::utc())
  io::print(datetime::toIso(dt, 0))
END SUB
```

prints:

```
2026-06-26T01:02:03Z
```"#;

// bug-521. The one-argument form now DELEGATES rather than carrying its own
// copy of the pattern: identical output is guaranteed by construction, which is
// the property the fix has to preserve (every acceptance golden with a
// timestamp in it reads this member's output).
#[rustfmt::skip]
const BODY_1: &str =
r#"FUNC __datetime_toIso(dt AS DateTime) AS String
  RETURN __datetime_toIso2(dt, 3)
END FUNC"#;

#[rustfmt::skip]
const BODY_2: &str =
r#"FUNC __datetime_toIso2(dt AS DateTime, digits AS Integer) AS String
  IF digits <> 0 AND digits <> 3 AND digits <> 6 AND digits <> 9 THEN
    FAIL error(77050002, "datetime: iso fraction digits must be 0, 3, 6 or 9")
  END IF
  LET stem AS String = __datetime_padN(dt.date.year, 4) & "-" & __datetime_pad2(dt.date.month) & "-" & __datetime_pad2(dt.date.day) & "T" & __datetime_pad2(dt.time.hour) & ":" & __datetime_pad2(dt.time.minute) & ":" & __datetime_pad2(dt.time.second)
  IF digits = 0 THEN
    RETURN stem & __datetime_isoZone(dt.offset)
  END IF
  RETURN stem & "." & strings::left(__datetime_padN(dt.time.nanos, 9), digits) & __datetime_isoZone(dt.offset)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "toIso",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("DateTime[, Integer]"),
        internal_only: false,
        // Arity-dispatched, the shape `datetime::parse` already uses: 1 arg ->
        // `__datetime_toIso` (millisecond, unchanged), 2 args -> `__datetime_toIso2`.
        implementations: vec![
            super::Implementation {
                params: vec![super::Parameter {
                    name: "dt",
                    desc: "The date-time to render as ISO-8601.",
                    aliases: &[],
                    ty: super::ParameterType::named("DateTime"),
                    default: super::DefaultValue::None,
                }],
                return_type: super::ParameterType::String,
                errors: vec![],
                body: super::Body::mfb(BODY_1, "__datetime_toIso"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "dt",
                        desc: "The date-time to render as ISO-8601.",
                        aliases: &[],
                        ty: super::ParameterType::named("DateTime"),
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "digits",
                        desc: "How many fractional-second digits to emit: 0 (no fractional field at all), 3 (milliseconds, what the one-argument form emits), 6 (microseconds), or 9 (nanoseconds — the only lossless width). Any other value raises `ErrInvalidArgument`.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::String,
                errors: vec![],
                body: super::Body::mfb(BODY_2, "__datetime_toIso2"),
            },
        ],
    });
}
