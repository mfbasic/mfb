//! `datetime::withZone` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).
//!
//! bug-518: the `zone` parameter's `desc` used to claim the opposite of the
//! `BODY` two screens below — that `withZone` "reinterprets the same wall-clock
//! reading … so it names a different instant" — and sent the reader to
//! `datetime::inZone`, which takes an `Instant`, not a `DateTime`, so the advice
//! did not even typecheck as a substitution. `inZone(resolve(dt), z)` preserves
//! the instant; the Description, the composition paragraph and both examples all
//! said so. The prose was the defect, and the operation the wrong sentence
//! described is `datetime::civil(dt.date, dt.time, zone)`, which the row now
//! names. `datetime-withzone-instant-rt` pins the property so a future change to
//! the BODY breaks a test rather than quietly making the old prose true.

const INTRO: &str = r#"Re-project a `datetime::DateTime` into a different `datetime::Zone`, preserving the absolute instant."#;
const DESC: &str = r#"`datetime::withZone` returns the civil `datetime::DateTime` that an observer in `zone`
reads at the very same absolute moment named by `dt`. The underlying point on the
UTC timeline is unchanged; only the wall-clock fields, the carried `zone`, and the
resolved UTC offset are re-derived for the new zone.

The function is exactly the composition of `datetime::resolve` and
`datetime::inZone`: it collapses `dt` back to a `datetime::Instant` with `datetime::resolve`
and then projects that `datetime::Instant` into `zone` with `datetime::inZone`.


The `resolve` step reaches the UTC timeline using the offset already pinned on
`dt`, with no zone lookup at all. The `inZone` step then works out the offset
`zone` had at that instant — zero for a UTC zone (`datetime::ZoneKind.Utc`), the stored
constant for a fixed-offset zone (`datetime::ZoneKind.FixedOffset`, built with
`datetime::fixedOffset`), and the DST-correct host offset for a local zone
(`datetime::ZoneKind.Local`, built with `datetime::local`) — and produces the civil date
and time an observer in `zone` reads at that moment.



The returned `datetime::DateTime` carries the new civil date and time, `zone` itself, and
the offset resolved for `zone`. The sub-second `nanos` field is carried through
both steps verbatim, so it equals `dt.time.nanos`. Because the instant is
preserved, `datetime::resolve` on the result returns the same `datetime::Instant` as
`datetime::resolve` on `dt`: `withZone` is an identity on the absolute moment and
changes only its civil presentation. It is pure for UTC and fixed-offset zones;
for a local zone it reads the host's time-zone configuration through the
`datetime::localOffset` OS intrinsic to resolve the offset."#;
const EX: &str = r#"Re-project a UTC `datetime::DateTime` into a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET z AS datetime::Zone = datetime::fixedOffset(5, 30)
  LET shifted AS datetime::DateTime = datetime::withZone(dt, z)
END SUB
```

Convert a `datetime::DateTime` to the host's local zone without changing the instant:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::inZone(datetime::now(), datetime::utc())
  LET local AS datetime::DateTime = datetime::withZone(dt, datetime::local())
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
                    desc: "The date-time whose zone to change.",
                    aliases: &[],
                    ty: super::ParameterType::named("DateTime"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "zone",
                    desc: "The zone to read the same moment in. The instant is **preserved**: the civil fields and the offset are re-derived for this zone, so the wall-clock reading changes but the point on the UTC timeline does not. To do the opposite — keep `dt`'s wall-clock reading and let it name a different instant in this zone — use `datetime::civil(dt.date, dt.time, zone)`.",
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
