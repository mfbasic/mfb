//! `datetime::parseIso` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Parse an RFC 3339 / ISO 8601 timestamp into a `datetime::DateTime`."#;
const DESC: &str = r#"`datetime::parseIso` reads an RFC 3339 (ISO 8601 profile) timestamp from `value`
and returns the `datetime::DateTime` it names. It is the convenience inverse of
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
  any digits beyond the ninth are read but ignored
- `<offset>` — required UTC offset: `Z` or `z` for UTC, otherwise a signed
  `+/-HH:MM` or `+/-HHMM` (the colon between offset hours and minutes is optional)

The numeric readers are greedy up to their stated width but also accept fewer
digits, so a field may be written with or without leading padding as long as the
surrounding separators are present. The offset is mandatory; unlike
`datetime::parse` there is no zone argument and no defaulting to UTC, because a
conforming RFC 3339 timestamp always carries its own offset. The parsed offset is
applied directly, making the result a fixed-offset moment.

Like `datetime::parse`, `parseIso` range-checks the decoded calendar fields
against the bounds `datetime::date` and `datetime::time` enforce: `month` in
`1 .. 12`, `day` in `1 ..` the length of that month in that year, `HH` in
`0 .. 23`, `mm` and `ss` in `0 .. 59`. An out-of-range component such as month
13 or day 40 raises `ErrInvalidFormat` rather than being carried into the
resulting `datetime::DateTime` or rolled over into a different date. The offset's
magnitude must be under 24 hours. `parseIso` is pure: it reads no host state and
has no side effects."#;
const EX: &str = r#"Parse a UTC timestamp:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::parseIso("1969-07-20T20:17:00Z")
END SUB
```

Parse a fractional second with a positive offset:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::parseIso("2026-06-25T14:30:00.250+05:30")
END SUB
```

A space may stand in for the `T` separator:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::parseIso("2026-06-26 09:30:00-08:00")
END SUB
```

Text that is missing its required offset is not valid RFC 3339 and raises
`ErrInvalidFormat`:

```
IMPORT datetime
IMPORT io

SUB main()
  LET bad AS datetime::DateTime = datetime::parseIso("2026-06-26T09:30:00")
  io::print("accepted")
  EXIT SUB
TRAP(err)
  io::print("rejected: " & toString(err.code))
  EXIT SUB
END TRAP
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_parseIso(value AS String) AS DateTime
  LET n AS Integer = len(value)
  LET yr AS __datetime_NumRead = __datetime_readNum(value, 0, 4)
  MUT pos AS Integer = yr.nextPos
  pos = __datetime_expect(value, pos, "-")
  LET mo AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
  pos = __datetime_expect(value, mo.nextPos, "-")
  LET dy AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
  pos = dy.nextPos
  LET sep AS String = strings::mid(value, pos, 1)
  IF sep <> "T" AND sep <> "t" AND sep <> " " THEN
    FAIL error(77050003, "datetime: expected date/time separator")
  END IF
  pos = pos + 1
  LET hh AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
  pos = __datetime_expect(value, hh.nextPos, ":")
  LET mm AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
  pos = __datetime_expect(value, mm.nextPos, ":")
  LET ss AS __datetime_NumRead = __datetime_readNum(value, pos, 2)
  pos = ss.nextPos
  MUT nanos AS Integer = 0
  IF pos < n AND strings::mid(value, pos, 1) = "." THEN
    pos = pos + 1
    MUT frac AS Integer = 0
    MUT digits AS Integer = 0
    WHILE pos < n AND __datetime_isDigit(strings::mid(value, pos, 1)) AND digits < 9
      frac = frac * 10 + toInt(strings::mid(value, pos, 1))
      digits = digits + 1
      pos = pos + 1
    END WHILE
    WHILE digits < 9
      frac = frac * 10
      digits = digits + 1
    END WHILE
    nanos = frac
    WHILE pos < n AND __datetime_isDigit(strings::mid(value, pos, 1))
      pos = pos + 1
    END WHILE
  END IF
  LET off AS __datetime_NumRead = __datetime_readOffset(value, pos)
  __datetime_checkFields(yr.value, mo.value, dy.value, hh.value, mm.value, ss.value, nanos)
  LET d AS Date = Date[yr.value, mo.value, dy.value]
  LET t AS Time = Time[hh.value, mm.value, ss.value, nanos]
  RETURN DateTime[d, t, __datetime_fixedOffset1(off.value), off.value]
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "parseIso",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "value",
                desc: "The ISO-8601 text to parse.",
                aliases: &[],
                ty: super::ParameterType::String,
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_parseIso"),
        }],
    });
}
