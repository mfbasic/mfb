//! `datetime::local` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"The `Zone` representing the host's local time."#;
const DESC: &str = r#"`datetime::local` returns the `Zone` that represents the host's local time. The
returned `Zone` has kind `ZoneKind.Local`, marking it as the host-resolved local
zone rather than the UTC zone built by `datetime::utc` (`ZoneKind.Utc`) or an
arbitrary fixed offset built by `datetime::fixedOffset`
(`ZoneKind.FixedOffset`).


Unlike `datetime::utc` and `datetime::fixedOffset`, whose offsets are baked into
the `Zone` at construction, the local zone holds no fixed offset of its own. The
`Zone` returned here carries the label `"Local"` and no offset of its own; the
real offset is worked out per-instant from the host's time-zone rules when the
zone is applied to a particular moment. Projecting an `Instant`
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
the zone is resolved against an instant during projection."#;
const EX: &str = r#"Obtain the local zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::local()
END SUB
```

Project the current instant into the local zone to read its civil fields:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET here AS DateTime = datetime::inZone(t, datetime::local())
END SUB
```

Combine a date and time into a `DateTime` in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_local AS Zone
  RETURN Zone[0, 2, "Local"]
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "local",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::named("Zone"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_local"),
        }],
    });
}
