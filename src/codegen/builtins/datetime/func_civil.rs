//! `datetime::civil` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Build a zoned `datetime::DateTime` from a civil `datetime::Date`, `datetime::Time`, and `datetime::Zone`."#;
const DESC: &str = r#"`datetime::civil` builds a `datetime::DateTime` by reading a calendar `date` and a
wall-clock `time` as a local time in `zone`, resolving the UTC offset that
applies to that local moment, and returning the canonical projection of the
resulting `datetime::Instant` back through `zone`. Because the result is the projection of
a concrete `datetime::Instant`, it round-trips: `datetime::resolve` on the returned
`datetime::DateTime` recovers the same `datetime::Instant`, and that `datetime::Instant` projected through
`zone` with `datetime::inZone` reproduces the same `datetime::DateTime` fields.


The `year`, `month`, and `day` of `date` and the `hour`, `minute`, and `second`
of `time` are read together as one wall-clock moment, treated as a civil
(zone-local) time. The offset for that moment is then resolved from
`zone`. For a zone with a fixed offset (built by `datetime::utc` or
`datetime::fixedOffset`) the offset is constant; for the host's local zone
(`datetime::local`) it is resolved from the platform's zone table at that
instant, so the result is daylight-saving correct.


When the named local time does not exist or is not unique because of a
daylight-saving transition, `civil` resolves it deterministically. It probes the
zone's offset one day before and one day after the named local time to bracket
any single nearby transition. If both probes agree, that offset is used
directly. If they differ, a spring-forward gap (the named local time is skipped)
shifts forward onto the post-transition offset, and a fall-back overlap (the
named local time occurs twice) takes the earlier, pre-transition offset.


The sub-second `nanos` of `time` are carried through unchanged into the
resulting `datetime::Instant` and `datetime::DateTime`; only the whole-second civil fields
participate in offset resolution. `civil` is pure: beyond what `zone` itself
resolves it reads no host state and has no side effects."#;
const EX: &str = r#"Combine a date and time into a `datetime::DateTime` in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS datetime::Date = datetime::date(2026, 6, 26)
  LET tm AS datetime::Time = datetime::time(9, 30)
  LET dt AS datetime::DateTime = datetime::civil(d, tm, datetime::local())
END SUB
```

Build a `datetime::DateTime` in UTC and recover its `datetime::Instant`:

```
IMPORT datetime

SUB main()
  LET d AS datetime::Date = datetime::date(2026, 1, 1)
  LET tm AS datetime::Time = datetime::time(0, 0)
  LET dt AS datetime::DateTime = datetime::civil(d, tm, datetime::utc())
  LET at AS datetime::Instant = datetime::resolve(dt)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_civil(d AS Date, t AS Time, z AS Zone) AS DateTime
  LET localSeconds AS Integer = __datetime_daysFromCivil(d.year, d.month, d.day) * 86400 + t.hour * 3600 + t.minute * 60 + t.second
  LET epochSeconds AS Integer = __datetime_resolveLocal(localSeconds, z)
  RETURN __datetime_inZone(Instant[epochSeconds, t.nanos], z)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "civil",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Date, Time, Zone"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "date",
                    desc: "The calendar date.",
                    aliases: &[],
                    ty: super::ParameterType::named("Date"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "time",
                    desc: "The wall-clock time on that date.",
                    aliases: &[],
                    ty: super::ParameterType::named("Time"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "zone",
                    desc: "The zone the wall-clock time is read in. This is what decides which instant the pair names; the same date and time in two zones are two different instants.",
                    aliases: &[],
                    ty: super::ParameterType::named("Zone"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_civil"),
        }],
    });
}
