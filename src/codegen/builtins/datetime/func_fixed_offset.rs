//! `datetime::fixedOffset` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

#[rustfmt::skip]
const BODY_1: &str =
r#"FUNC __datetime_fixedOffset1(offsetSeconds AS Integer) AS Zone
  IF offsetSeconds <= -86400 OR offsetSeconds >= 86400 THEN
    FAIL error(77050002, "datetime: fixed offset magnitude must be under 24h")
  END IF
  RETURN Zone[offsetSeconds, 1, __datetime_offsetLabel(offsetSeconds)]
END FUNC"#;

#[rustfmt::skip]
const BODY_2: &str =
r#"FUNC __datetime_fixedOffset2(hours AS Integer, mins AS Integer) AS Zone
  IF mins < 0 OR mins > 59 THEN
    FAIL error(77050002, "datetime: fixed offset minutes out of range")
  END IF
  MUT absHours AS Integer = hours
  MUT negative AS Boolean = FALSE
  IF hours < 0 THEN
    absHours = -hours
    negative = TRUE
  END IF
  MUT total AS Integer = absHours * 3600 + mins * 60
  IF negative THEN
    total = -total
  END IF
  RETURN __datetime_fixedOffset1(total)
END FUNC"#;

const INTRO: &str = r#"Build a `datetime::Zone` with a constant UTC offset."#;
const DESC: &str = r#"`datetime::fixedOffset` builds a `datetime::Zone` whose offset from UTC is a constant
value that does not vary with the instant being projected. Unlike
`datetime::local`, whose offset is resolved against the host's configured time
zone, and unlike `datetime::utc`, the canonical zero-offset zone, a
fixed-offset `datetime::Zone` carries a single signed offset that applies to every
`datetime::Instant` projected through it. The returned `datetime::Zone` has a zone kind of
`datetime::ZoneKind::FixedOffset` and a label rendered as `+HH:MM` or `-HH:MM`, with `:SS`
appended when the offset is not a whole number of minutes.


A `datetime::Zone` is the bridge between the absolute UTC timeline (a `datetime::Instant`) and the
human-readable civil fields of a `datetime::DateTime`. Projecting a `datetime::Instant` through a
fixed-offset zone with `datetime::inZone` produces a `datetime::DateTime` whose year,
month, day, and time fields are shifted from UTC by exactly the offset this
function encodes: a positive offset places the civil fields ahead of UTC (east
of the prime meridian), a negative offset places them behind UTC (west).

The one-argument form takes the offset directly as a raw signed second count.
The two-argument form takes whole `hours` and a `mins` magnitude in the range
`0 .. 59`. `hours` alone carries the sign and `mins` is never negative: the total
is `abs(hours) * 3600 + mins * 60` seconds, negated when `hours` is negative.
Thus `datetime::fixedOffset(-5, 30)` is `-05:30` (five hours and thirty minutes
behind UTC), and `datetime::fixedOffset(5, 30)` is `+05:30`. When `hours` is `0`
the offset is positive, so `datetime::fixedOffset(0, 30)` is `+00:30`, and a
negative `mins` such as `datetime::fixedOffset(0, -30)` raises
`ErrInvalidArgument`. A zone less than an hour west of UTC can only be built with
the one-argument form: `datetime::fixedOffset(-1800)` is `-00:30`.


In both forms the offset magnitude must be strictly under 24 hours (86400
seconds); an offset of exactly `+/-24h` or more raises `ErrInvalidArgument`, as
does `mins` outside `0 .. 59`. In the two-argument form an `hours` so large that
`hours * 3600` does not fit an `Integer` raises `ErrOverflow`. The label is
`+HH:MM`, or `+HH:MM:SS` when the offset is not a whole number of minutes. The
function is pure: it reads no host state and has no side effects."#;
const EX: &str = r#"Build a zone five and a half hours behind UTC:

```
IMPORT datetime

SUB main()
  LET z AS datetime::Zone = datetime::fixedOffset(-5, 30)
END SUB
```

Build the same zone from a raw second count:

```
IMPORT datetime

SUB main()
  LET z AS datetime::Zone = datetime::fixedOffset(-19800)
END SUB
```

Project the current instant into a fixed `+09:00` zone:

```
IMPORT datetime

SUB main()
  LET t AS datetime::Instant = datetime::now()
  LET local AS datetime::DateTime = datetime::inZone(t, datetime::fixedOffset(9, 0))
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
                    desc: "The offset from UTC in seconds, -86399 through 86399. Positive is east of UTC, negative is west, and 0 is UTC.",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                }],
                return_type: super::ParameterType::named("Zone"),
                errors: vec!["ErrInvalidArgument"],
                body: super::Body::mfb(BODY_1, "__datetime_fixedOffset1"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "hours",
                        desc: "The whole-hour part of the offset, -23 through 23. Negative for zones west of UTC.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "mins",
                        desc: "The minutes part of the offset, 0 through 59. Never negative: `hours` carries the sign, so `(-5, 30)` is -05:30. A zone under an hour west of UTC needs the one-argument form.",
                        aliases: &[],
                        ty: super::ParameterType::Integer,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::named("Zone"),
                errors: vec!["ErrInvalidArgument", "ErrOverflow"],
                body: super::Body::mfb(BODY_2, "__datetime_fixedOffset2"),
            },
        ],
    });
}
