//! `datetime::instant` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

#[rustfmt::skip]
const BODY_1: &str =
r#"FUNC __datetime_instant1(seconds AS Integer) AS Instant
  RETURN Instant[seconds, 0]
END FUNC"#;

#[rustfmt::skip]
const BODY_2: &str =
r#"FUNC __datetime_instant2(seconds AS Integer, nanos AS Integer) AS Instant
  RETURN __datetime_normInstant(seconds, nanos)
END FUNC"#;

#[rustfmt::skip]
const BODY_3: &str =
r#"FUNC __datetime_instant3(mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant
  RETURN __datetime_normInstant(mins * 60 + seconds, nanos)
END FUNC"#;

#[rustfmt::skip]
const BODY_4: &str =
r#"FUNC __datetime_instant4(hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant
  RETURN __datetime_normInstant(hours * 3600 + mins * 60 + seconds, nanos)
END FUNC"#;

#[rustfmt::skip]
const BODY_5: &str =
r#"FUNC __datetime_instant5(days AS Integer, hours AS Integer, mins AS Integer, seconds AS Integer, nanos AS Integer) AS Instant
  RETURN __datetime_normInstant(days * 86400 + hours * 3600 + mins * 60 + seconds, nanos)
END FUNC"#;

const INTRO: &str =
    r#"Build a `datetime::Instant` from seconds, nanoseconds, or larger time components."#;
const DESC: &str = r#"`datetime::instant` builds a `datetime::Instant` on the UTC timeline (the Unix epoch,
without leap seconds) at a given offset after `1970-01-01T00:00:00Z`. The result
carries whole seconds since the epoch in its `seconds` field and a sub-second
remainder in its `nanos` field, normalized into the range `0 .. 999_999_999`.

`instant` is overloaded by argument count, with five disjoint forms selected by
the number of `Integer` arguments (one through five).
 The one- and two-argument forms take
whole seconds and, optionally, a nanosecond adjustment. The three-, four-, and
five-argument forms are component builders that fold larger units down into a
single second count: the three-argument form computes `mins*60 + seconds`, the
four-argument form adds `hours*3600`, and the five-argument form adds
`days*86400`, in every case adding the trailing `nanos` last.


Whichever form is used (except the one-argument form), the supplied seconds and
nanos are normalized: any whole seconds embedded in `nanos` are carried into the
`seconds` field, and a negative `nanos` value borrows a second so the stored
`nanos` always lands in `0 .. 999_999_999`.
 Every numeric
argument may be negative, which selects an instant before the epoch. The
one-argument form performs no normalization because its `nanos` is fixed at zero.


`instant` is overloaded, so every parameter of the form you call must be supplied
explicitly; the component forms carry no defaults.
 The folding and
normalization are ordinary signed `Integer` arithmetic, so a sufficiently large
day, hour, minute, or second magnitude can overflow the `Integer` range and trap.
To shift an existing `datetime::Instant` by a span rather than build one from scratch, use
`datetime::add` or `datetime::subtract` with a `datetime::Duration`. `instant` is pure: the
same arguments always yield the same `datetime::Instant`, and it has no side effects."#;
const EX: &str = r#"Build a `datetime::Instant` from a whole-second epoch offset:

```
IMPORT datetime

SUB main()
  LET t AS datetime::Instant = datetime::instant(1_700_000_000)
END SUB
```

Build a `datetime::Instant` with a sub-second adjustment that normalizes into the `seconds`
field:

```
IMPORT datetime

SUB main()
  LET t AS datetime::Instant = datetime::instant(10, 1_500_000_000)
END SUB
```

Build a `datetime::Instant` from day, hour, minute, second, and nanosecond components:

```
IMPORT datetime

SUB main()
  LET t AS datetime::Instant = datetime::instant(1, 2, 3, 4, 0)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "instant",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("1 to 5 Integer"),
        internal_only: false,
        implementations: vec![
            super::Implementation {
                params: vec![super::Parameter {
                    name: "seconds",
                    desc: "Whole seconds since the Unix epoch, 1970-01-01T00:00:00Z.",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                }],
                return_type: super::ParameterType::named("Instant"),
                errors: vec![],
                body: super::Body::mfb(BODY_1, "__datetime_instant1"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "seconds",
                        desc: "Whole seconds since the Unix epoch, 1970-01-01T00:00:00Z.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "nanos",
                        desc: "Nanoseconds past the second.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::named("Instant"),
                errors: vec![],
                body: super::Body::mfb(BODY_2, "__datetime_instant2"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "mins",
                        desc: "Whole minutes.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "seconds",
                        desc: "Whole seconds since the Unix epoch, 1970-01-01T00:00:00Z.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "nanos",
                        desc: "Nanoseconds past the second.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::named("Instant"),
                errors: vec![],
                body: super::Body::mfb(BODY_3, "__datetime_instant3"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "hours",
                        desc: "Whole hours.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "mins",
                        desc: "Whole minutes.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "seconds",
                        desc: "Whole seconds since the Unix epoch, 1970-01-01T00:00:00Z.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "nanos",
                        desc: "Nanoseconds past the second.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::named("Instant"),
                errors: vec![],
                body: super::Body::mfb(BODY_4, "__datetime_instant4"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "days",
                        desc: "Whole days since the epoch.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "hours",
                        desc: "Whole hours.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "mins",
                        desc: "Whole minutes.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "seconds",
                        desc: "Whole seconds since the Unix epoch, 1970-01-01T00:00:00Z.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "nanos",
                        desc: "Nanoseconds past the second.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::named("Instant"),
                errors: vec![],
                body: super::Body::mfb(BODY_5, "__datetime_instant5"),
            },
        ],
    });
}
