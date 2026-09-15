//! `datetime::resolve` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str =
    r#"Collapse a civil `datetime::DateTime` back to the absolute `datetime::Instant` it names."#;
const DESC: &str = r#"`datetime::resolve` is the inverse of `datetime::inZone`: where `inZone` projects
an absolute instant onto the wall-clock fields an observer in a zone reads,
`resolve` collapses those wall-clock fields — together with the UTC offset already
pinned on `dt` — back onto the single point on the UTC timeline they denote.

The computation needs no zone lookup: `resolve` combines the stored civil date,
time, and UTC offset (`dt.offset`) into an absolute instant, and pairs it with
`dt.time.nanos`.


Because the offset is read directly from `dt` rather than re-derived from the
zone, `resolve` is unambiguous even across daylight-saving transitions: it
reproduces exactly the instant a `datetime::DateTime` was built from. For any instant `at`
and zone `z`, `datetime::resolve(datetime::inZone(at, z))` returns `at` unchanged.
The `seconds` field participates in the date/time arithmetic; the `nanos` field is
copied through verbatim. A `datetime::DateTime` from the constructors always
resolves. A `datetime::DateTime` record you build yourself is not validated: `resolve`
uses its stored fields and offset as given, so month 13 or hour 99 still yields an
instant. `ErrOverflow` is raised if converting the date and time to seconds, or
subtracting the stored offset, leaves the `Integer` range. `resolve` is pure and reads no host state."#;
const EX: &str = r#"Round-trip an instant through a civil `datetime::DateTime` and back:

```
IMPORT datetime

SUB main()
  LET at AS datetime::Instant = datetime::now()
  LET dt AS datetime::DateTime = datetime::inZone(at, datetime::utc())
  LET back AS datetime::Instant = datetime::resolve(dt)
END SUB
```

Resolve a civil `datetime::DateTime` built in a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET z AS datetime::Zone = datetime::fixedOffset(5, 30)
  LET dt AS datetime::DateTime = datetime::inZone(datetime::now(), z)
  LET at AS datetime::Instant = datetime::resolve(dt)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_resolve(dt AS DateTime) AS Instant
  LET localSeconds AS Integer = __datetime_daysFromCivil(dt.date.year, dt.date.month, dt.date.day) * 86400 + dt.time.hour * 3600 + dt.time.minute * 60 + dt.time.second
  RETURN Instant[localSeconds - dt.offset, dt.time.nanos]
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "resolve",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("DateTime"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "dt",
                desc: "The date-time to resolve. `resolve` uses its stored UTC offset directly; an ambiguous or non-existent local time is settled earlier, by `datetime::civil`.",
                aliases: &[],
                ty: super::ParameterType::named("DateTime"),
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::named("Instant"),
            errors: vec!["ErrOverflow"],
            body: super::Body::mfb(BODY, "__datetime_resolve"),
        }],
    });
}
