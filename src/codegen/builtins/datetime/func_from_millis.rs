//! `datetime::fromMillis` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/fromMillis.md`.

const INTRO: &str = r#"Build the `Instant` at a given epoch-millisecond count."#;
const DESC: &str = r#"`datetime::fromMillis` builds an `Instant` on the UTC timeline (Unix epoch,
leap-second-free) from a single count of whole milliseconds measured from
`1970-01-01T00:00:00Z`. A `millis` of `0` yields the epoch itself, positive
values select instants after the epoch, and negative values select instants
before it.

The count is split into a whole-second `seconds` field and a sub-second `nanos`
field by *floor* division, so the `nanos` remainder is always non-negative. The
implementation first computes the toward-zero quotient `millis / 1000` and
remainder `millis MOD 1000`; when that remainder is negative it adds `1000` to
the remainder and subtracts `1` from the quotient, borrowing one second. The
`seconds` field is therefore the mathematical floor of `millis / 1000` and the
`nanos` field is the borrowed, non-negative millisecond remainder scaled to
nanoseconds (`remainder * 1000000`), always in `0..999000000`. A `millis` of
`-1` produces `seconds` `-1` and `nanos` `999000000`, the instant one
millisecond before the epoch. Because the input carries only millisecond
resolution, the `nanos` field is always a whole number of milliseconds — its
microsecond and nanosecond digits are zero.


The arithmetic cannot overflow: dividing by `1000` only reduces the magnitude of
the `seconds` field, and the scaled remainder never exceeds `999000000`, so the
result is always representable. `datetime::fromMillis` is pure: it reads no host
state and the same `millis` always yields the same `Instant`.

`datetime::fromMillis` is the inverse of `datetime::toMillis` to
whole-millisecond precision. Because the input has no sub-millisecond component,
round-tripping an arbitrary `Instant` through `datetime::toMillis` and back loses
its microsecond and nanosecond digits; for full nanosecond precision use
`datetime::toNanos` together with `datetime::instant`."#;
const EX: &str = r#"Build an `Instant` from an epoch-millisecond timestamp:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::fromMillis(1_700_000_000_000)
END SUB
```

Select the instant one millisecond before the epoch:

```
IMPORT datetime

SUB main()
  LET before AS Instant = datetime::fromMillis(-1)
END SUB
```

Round-trip an instant through its millisecond count:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET ms AS Integer = datetime::toMillis(at)
  LET back AS Instant = datetime::fromMillis(ms)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_fromMillis(millis AS Integer) AS Instant
  MUT q AS Integer = millis / 1000
  MUT r AS Integer = millis MOD 1000
  IF r < 0 THEN
    r = r + 1000
    q = q - 1
  END IF
  RETURN Instant[q, r * 1000000]
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "fromMillis",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: super::arg_hint("fromMillis"),
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "millis",
                desc: "",
                aliases: &[],
                ty: super::ParameterType::Integer,
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::Named("Instant"),
            errors: vec![],
            lowering: super::Lowering::Helper,
            body: super::Body::mfb(BODY, "__datetime_fromMillis"),
        }],
    });
}
