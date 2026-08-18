//! `datetime::fixedOffset` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`.

const INTRO: &str = r#"Build a `Zone` with a constant UTC offset."#;
const DESC: &str = r#"`datetime::fixedOffset` builds a `Zone` whose offset from UTC is a constant
value that does not vary with the instant being projected. Unlike
`datetime::local`, whose offset is resolved against the host's configured time
zone, and unlike `datetime::utc`, the canonical zero-offset zone, a
fixed-offset `Zone` carries a single signed offset that applies to every
`Instant` projected through it. The returned `Zone` has a zone kind of
`ZoneKind::FixedOffset` and a label rendered in the form `+HH:MM` or `-HH:MM`.


A `Zone` is the bridge between the absolute UTC timeline (an `Instant`) and the
human-readable civil fields of a `DateTime`. Projecting an `Instant` through a
fixed-offset zone with `datetime::inZone` produces a `DateTime` whose year,
month, day, and time fields are shifted from UTC by exactly the offset this
function encodes: a positive offset places the civil fields ahead of UTC (east
of the prime meridian), a negative offset places them behind UTC (west).

The one-argument form takes the offset directly as a raw signed second count.
The two-argument form takes whole `hours` and a `mins` magnitude in the range
`0 .. 59`; `mins` contributes its magnitude only and inherits the sign of
`hours`. Thus `datetime::fixedOffset(-5, 30)` is `-05:30` (five hours and
thirty minutes behind UTC), and `datetime::fixedOffset(5, 30)` is `+05:30`. The
two-argument form is implemented in terms of the one-argument form by combining
the hours and minutes into a total second count of
`sign(hours) * (abs(hours) * 3600 + mins * 60)`.


In both forms the offset magnitude must be strictly under 24 hours (86400
seconds); an offset of exactly `+/-24h` or more is rejected. The function is
pure: it reads no host state and has no side effects."#;
const EX: &str = r#"Build a zone five and a half hours behind UTC:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-5, 30)
END SUB
```

Build the same zone from a raw second count:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-19800)
END SUB
```

Project the current instant into a fixed `+09:00` zone:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET local AS DateTime = datetime::inZone(t, datetime::fixedOffset(9, 0))
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "fixedOffset",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer[, Integer]"),
        internal_only: false,
        implementations: vec![
            super::Implementation {
                params: vec![super::Parameter {
                    name: "offsetSeconds",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                }],
                return_type: super::ParameterType::Named("Zone"),
                errors: vec![],
                body: super::Body::Rewrite("__datetime_fixedOffset1"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "hours",
                        desc: "",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "mins",
                        desc: "",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::Named("Zone"),
                errors: vec![],
                body: super::Body::Rewrite("__datetime_fixedOffset2"),
            },
        ],
    });
}
