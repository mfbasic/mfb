//! `datetime::utc` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"The `datetime::Zone` representing Coordinated Universal Time."#;
const DESC: &str = r#"`datetime::utc` returns the `datetime::Zone` that represents Coordinated Universal Time: a
fixed zone whose offset from UTC is a constant zero seconds and whose label is
the literal string `"UTC"`. The returned `datetime::Zone` carries a zone kind of
`datetime::ZoneKind::Utc` (the first `datetime::ZoneKind` variant, tag `0`), marking it as the
canonical UTC zone rather than an arbitrary fixed offset built with
`datetime::fixedOffset` (kind `datetime::ZoneKind::FixedOffset`).


A `datetime::Zone` is the bridge between the absolute UTC timeline (a `datetime::Instant`) and the
human-readable civil fields of a `datetime::DateTime`. Project a `datetime::Instant` through this
zone with `datetime::inZone` to obtain a `datetime::DateTime` whose year, month, day, and
time fields are expressed in UTC; `datetime::toUtc` is the dedicated shorthand
for exactly that projection. Because the offset is always zero, the civil fields
of a `datetime::DateTime` in this zone match the seconds-since-epoch of the originating
`datetime::Instant` directly, with no offset adjustment.

`datetime::utc` takes no arguments and always returns the same constant `datetime::Zone`.
It is pure: every call yields an identical UTC zone, it reads no host state, and
it has no side effects. Unlike `datetime::local`, whose offset depends on the
host's configured time zone, `datetime::utc` is wholly independent of the
environment."#;
const EX: &str = r#"Obtain the UTC zone:

```
IMPORT datetime

SUB main()
  LET z AS datetime::Zone = datetime::utc()
END SUB
```

Project the current instant into UTC to read its civil fields:

```
IMPORT datetime

SUB main()
  LET t AS datetime::Instant = datetime::now()
  LET inUtc AS datetime::DateTime = datetime::inZone(t, datetime::utc())
END SUB
```

Combine a date and time into a UTC-zoned `datetime::DateTime`:

```
IMPORT datetime

SUB main()
  LET d AS datetime::Date = datetime::date(2026, 6, 26)
  LET tm AS datetime::Time = datetime::time(9, 30)
  LET dt AS datetime::DateTime = datetime::civil(d, tm, datetime::utc())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_utc AS Zone
  RETURN Zone[0, 0, "UTC"]
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "utc",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::named("Zone"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_utc"),
        }],
    });
}
