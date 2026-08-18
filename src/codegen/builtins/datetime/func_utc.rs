//! `datetime::utc` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`.

const INTRO: &str = r#"The `Zone` representing Coordinated Universal Time."#;
const DESC: &str = r#"`datetime::utc` returns the `Zone` that represents Coordinated Universal Time: a
fixed zone whose offset from UTC is a constant zero seconds and whose label is
the literal string `"UTC"`. The returned `Zone` carries a zone kind of
`ZoneKind::Utc` (the first `ZoneKind` variant, tag `0`), marking it as the
canonical UTC zone rather than an arbitrary fixed offset built with
`datetime::fixedOffset` (kind `ZoneKind::FixedOffset`).


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
environment."#;
const EX: &str = r#"Obtain the UTC zone:

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
            return_type: super::ParameterType::Named("Zone"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_utc"),
        }],
    });
}
