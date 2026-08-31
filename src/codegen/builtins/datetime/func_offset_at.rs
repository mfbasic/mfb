//! `datetime::offsetAt` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str =
    r#"A `datetime::Zone`'s signed UTC offset in seconds at a given `datetime::Instant`."#;
const DESC: &str = r#"`datetime::offsetAt` returns the signed offset from UTC, in seconds, that
`zone` applies to the absolute instant `at`. A positive result places the
zone's civil fields ahead of UTC (east of the prime meridian), a negative
result places them behind UTC (west), and zero means the zone coincides with
UTC at that instant. This is the exact quantity `datetime::inZone` adds to an
`datetime::Instant`'s seconds-since-epoch to produce the civil fields of a `datetime::DateTime`, so
`offsetAt` exposes that adjustment on its own.


How the offset is determined depends on the zone's kind. For a UTC zone
(`datetime::ZoneKind::Utc`) and a fixed-offset zone (`datetime::ZoneKind::FixedOffset`, built with
`datetime::fixedOffset`) the function returns the zone's stored constant offset
directly and does not consult `at` — the UTC zone stores zero, and a fixed zone
stores its single configured offset. For a local zone (`datetime::ZoneKind::Local`, built
with `datetime::local`, internally zone kind `2`) the offset is resolved
against the host's configured time zone for the specific instant `at`: it reads
the host zone table and is therefore DST-correct, returning the standard-time
offset for instants outside daylight saving and the shifted offset for instants
within it. Two calls with the same local zone but instants on opposite sides of
a DST transition can therefore return different values.


Only the `seconds` field of `at` participates; the sub-second `nanos` field is
ignored. The function reads no host state for UTC and fixed zones (those are
pure); for a local zone it reads the host's time-zone configuration through the
`datetime::localOffset` OS intrinsic."#;
const EX: &str = r#"A UTC zone always reports a zero offset:

```
IMPORT datetime

SUB main()
  LET off AS Integer = datetime::offsetAt(datetime::utc(), datetime::now())
END SUB
```

A fixed zone reports its constant offset regardless of the instant:

```
IMPORT datetime

SUB main()
  LET z AS datetime::Zone = datetime::fixedOffset(5, 30)
  LET off AS Integer = datetime::offsetAt(z, datetime::now())
END SUB
```

A local zone's offset is resolved DST-correctly for the given instant:

```
IMPORT datetime

SUB main()
  LET nowOff AS Integer = datetime::offsetAt(datetime::local(), datetime::now())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_offsetAt(z AS Zone, at AS Instant) AS Integer
  IF z.kind = 2 THEN
    RETURN datetime::localOffset(at.seconds)
  END IF
  RETURN z.offsetSeconds
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "offsetAt",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Zone, Instant"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "zone",
                    desc: "The zone to ask about.",
                    aliases: &[],
                    ty: super::ParameterType::named("Zone"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "at",
                    desc: "The instant to ask about. The offset depends on it wherever daylight saving applies.",
                    aliases: &[],
                    ty: super::ParameterType::named("Instant"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::Integer,
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_offsetAt"),
        }],
    });
}
