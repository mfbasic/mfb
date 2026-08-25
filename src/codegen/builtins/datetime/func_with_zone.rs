//! `datetime::withZone` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str =
    r#"Re-project a `DateTime` into a different `Zone`, preserving the absolute instant."#;
const DESC: &str = r#"`datetime::withZone` returns the civil `DateTime` that an observer in `zone`
reads at the very same absolute moment named by `dt`. The underlying point on the
UTC timeline is unchanged; only the wall-clock fields, the carried `zone`, and the
resolved UTC offset are re-derived for the new zone.

The function is exactly the composition of `datetime::resolve` and
`datetime::inZone`: it collapses `dt` back to an `Instant` with `datetime::resolve`
and then projects that `Instant` into `zone` with `datetime::inZone`.


The `resolve` step reads the offset already pinned on `dt` to reach the UTC
timeline without any zone lookup (`daysFromCivil(...) * 86400 + hour * 3600 +
minute * 60 + second - dt.offset`). The `inZone` step then resolves the effective
offset for `zone` at that instant — zero for a UTC zone (`ZoneKind::Utc`), the
stored constant for a fixed-offset zone (`ZoneKind::FixedOffset`, built with
`datetime::fixedOffset`), and the DST-correct host offset for a local zone
(`ZoneKind::Local`, built with `datetime::local`) — adds it to the instant's
seconds, floor-divides into whole days and second-of-day, and splits the result
into civil year/month/day and hour/minute/second with the proleptic Gregorian
calendar.



The returned `DateTime` carries the new civil date and time, `zone` itself, and
the offset resolved for `zone`. The sub-second `nanos` field is carried through
both steps verbatim, so it equals `dt.time.nanos`. Because the instant is
preserved, `datetime::resolve` on the result returns the same `Instant` as
`datetime::resolve` on `dt`: `withZone` is an identity on the absolute moment and
changes only its civil presentation. It is pure for UTC and fixed-offset zones;
for a local zone it reads the host's time-zone configuration through the
`datetime::localOffset` OS intrinsic to resolve the offset."#;
const EX: &str = r#"Re-project a UTC `DateTime` into a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET shifted AS DateTime = datetime::withZone(dt, z)
END SUB
```

Convert a `DateTime` to the host's local zone without changing the instant:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET local AS DateTime = datetime::withZone(dt, datetime::local())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_withZone(dt AS DateTime, z AS Zone) AS DateTime
  RETURN __datetime_inZone(__datetime_resolve(dt), z)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "withZone",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("DateTime, Zone"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "dt",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::named("DateTime"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "zone",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::named("Zone"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_withZone"),
        }],
    });
}
