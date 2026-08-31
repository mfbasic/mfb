//! `datetime::toUtc` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Project an absolute `Instant` into UTC to produce a civil `DateTime`."#;
const DESC: &str = r#"`datetime::toUtc` projects the absolute instant `at` into the UTC zone, yielding
the calendar date and wall-clock time that an observer reading UTC sees at that
moment. It is exactly shorthand for `datetime::inZone(at, datetime::utc())`: the
UTC zone contributes a zero offset, so the instant's seconds-since-epoch are
split directly — floor-divided into whole days and the second-of-day — into a
civil year/month/day (proleptic Gregorian calendar) and an
hour/minute/second-of-day, with no offset adjustment.


The returned `DateTime` carries four things: the civil date, the civil time, the
UTC zone, and a resolved offset of zero. Because the zero offset is pinned onto
the result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second
`nanos` field is preserved verbatim into the time's `nanos` field; only the
`seconds` field participates in the date and time computation, so an instant
before the Unix epoch (negative `seconds`) projects correctly.


Unlike `datetime::toLocal`, `datetime::toUtc` is pure: it reads no host
time-zone configuration and produces the same result on every platform. Because
the resolved offset is always zero, adding it to the instant's seconds cannot
overflow the `Integer` range, so this call raises no error of its own."#;
const EX: &str = r#"Project the current instant into UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
END SUB
```

Round-trip an instant through UTC and back:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET dt AS DateTime = datetime::toUtc(at)
  LET back AS Instant = datetime::resolve(dt)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_toUtc(at AS Instant) AS DateTime
  RETURN __datetime_inZone(at, __datetime_utc())
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "toUtc",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Instant"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "at",
                desc: "The instant to read in UTC.",
                aliases: &[],
                ty: super::ParameterType::named("Instant"),
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_toUtc"),
        }],
    });
}
