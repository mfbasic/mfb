//! `datetime::inZone` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Project an absolute `Instant` into a `Zone` to produce a civil `DateTime`."#;
const DESC: &str = r#"`datetime::inZone` is the primary "to civil time" call: it projects the absolute
instant `at` through `zone`, yielding the calendar date and wall-clock time that
an observer in that zone reads at that moment.

It first resolves the effective UTC offset for `zone` at the instant `at` — the
same quantity `datetime::offsetAt` returns: zero for a UTC zone
(`ZoneKind.Utc`), the stored constant for a fixed-offset zone
(`ZoneKind.FixedOffset`, built with `datetime::fixedOffset`), and the DST-correct
host offset for a local zone (`ZoneKind.Local`, built with `datetime::local`).
It then applies that offset to the instant to get the local calendar date and
wall-clock time, using the proleptic Gregorian calendar.


The returned `DateTime` carries four things: the civil date, the civil time,
`zone` itself, and the resolved offset. Because the offset is pinned onto the
result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second `nanos`
field is preserved verbatim into the time's `nanos` field; only the `seconds`
field participates in the offset and date/time computation, so an instant before
the Unix epoch (negative `seconds`) projects correctly.


`datetime::toUtc` and `datetime::toLocal` are shorthands for calling `inZone`
with the UTC zone and the host local zone, respectively. `inZone` is pure for UTC
and fixed-offset zones; for a local zone it reads the host's time-zone
configuration through the `datetime::localOffset` OS intrinsic to resolve the
offset."#;
const EX: &str = r#"Project the current instant into UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
END SUB
```

Project an instant into a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET dt AS DateTime = datetime::inZone(datetime::now(), z)
END SUB
```

Project into the host's local zone, with DST applied for that instant:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::local())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_inZone(at AS Instant, z AS Zone) AS DateTime
  LET off AS Integer = __datetime_offsetAt(z, at)
  LET localSeconds AS Integer = at.seconds + off
  LET days AS Integer = __datetime_floorDiv(localSeconds, 86400)
  LET secOfDay AS Integer = __datetime_floorMod(localSeconds, 86400)
  LET date AS Date = __datetime_civilFromDays(days)
  LET time AS Time = Time[secOfDay / 3600, (secOfDay / 60) MOD 60, secOfDay MOD 60, at.nanos]
  RETURN DateTime[date, time, z, off]
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "inZone",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Instant, Zone"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "at",
                    desc: "The instant to view. The instant itself does not change; only the zone it is read in does.",
                    aliases: &[],
                    ty: super::ParameterType::named("Instant"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "zone",
                    desc: "The zone to read it in.",
                    aliases: &[],
                    ty: super::ParameterType::named("Zone"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_inZone"),
        }],
    });
}
