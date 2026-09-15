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
daylight-saving transition, `civil` resolves it deterministically: a spring-forward gap (the named local time is skipped)
shifts forward onto the post-transition offset, and a fall-back overlap (the
named local time occurs twice) takes the earlier, pre-transition offset.


The sub-second `nanos` of `time` are carried through unchanged into the
resulting `datetime::Instant` and `datetime::DateTime`; only the whole-second civil fields
participate in offset resolution. `civil` has no side effects. With a
`datetime::utc` or `datetime::fixedOffset` zone its result depends only on its
arguments; with `datetime::local` it reads the host's zone rules, so the same date
and time can give a different result on a host set to another zone.

A date so far from the epoch that its second count does not fit an `Integer`
raises `ErrOverflow`. For a local zone, a wall-clock time outside the range the
host can convert raises `ErrInvalidArgument`, as `datetime::localOffset` does."#;
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
```

Resolve a local time on the two days a year when one does not exist, or exists
twice. Both cases below assume `TZ=America/New_York`; `datetime::local()` reads
the host zone, and there is no way to name a zone in the language yet, so the
zone is part of the example's premise rather than its code. The values shown are
measured, not derived from the policy:

```
IMPORT io
IMPORT datetime

SUB main()
  ' Spring forward. 2026-03-08 02:30 local is SKIPPED — the clock jumps from
  ' 01:59:59 -05:00 to 03:00:00 -04:00. A gap shifts forward onto the
  ' post-transition offset, so this names 03:30, not 01:30.
  LET gap AS datetime::DateTime = datetime::civil(datetime::date(2026, 3, 8), datetime::time(2, 30), datetime::local())
  io::print(datetime::format(gap, "yyyy-MM-dd HH:mm:ss ZZ"))
  ' 2026-03-08 03:30:00 -04:00

  ' Fall back. 2026-11-01 01:30 local happens TWICE, once at -04:00 and again
  ' an hour later at -05:00. An overlap takes the EARLIER, pre-transition
  ' offset, so this is the first 01:30.
  LET overlap AS datetime::DateTime = datetime::civil(datetime::date(2026, 11, 1), datetime::time(1, 30), datetime::local())
  io::print(datetime::format(overlap, "yyyy-MM-dd HH:mm:ss ZZ"))
  ' 2026-11-01 01:30:00 -04:00
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
                    desc: "The zone the wall-clock time is read in. Its offset decides which instant the pair names: the same date and time in zones with different offsets are different instants.",
                    aliases: &[],
                    ty: super::ParameterType::named("Zone"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec!["ErrInvalidArgument", "ErrOverflow"],
            body: super::Body::mfb(BODY, "__datetime_civil"),
        }],
    });
}
