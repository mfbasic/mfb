//! `datetime::negate` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`.

const INTRO: &str =
    r#"Return a `Duration` with the opposite sign — the additive inverse of a span."#;
const DESC: &str = r#"`datetime::negate` returns the additive inverse of `d`: the span of equal
magnitude that points the opposite way along a timeline. A forward span of `+90s`
becomes a backward span of `-90s`, a backward span becomes forward, and the zero
`Duration` negates to itself. Adding `d` to `datetime::negate(d)` yields a zero
span.

Negation acts on the whole span, not on each field independently. It negates both
the `seconds` and the `nanos` field, then re-normalizes so the stored `nanos`
always lands in the range `0 .. 999_999_999`, carrying any borrow into the
`seconds` field. So a `Duration` whose `seconds` is `0` and whose `nanos` is
`250_000_000` (a quarter second forward) negates to a `Duration` whose `seconds`
is `-1` and whose `nanos` is `750_000_000` — the same magnitude pointing
backward.

Negation is the same operation as `datetime::minus(zero, d)`. The arithmetic is
ordinary signed `Integer` arithmetic, so negating the most negative representable
`seconds` count has no positive counterpart in the `Integer` range and traps.
`negate` is pure: the same `Duration` always negates to the same result, and it
has no side effects."#;
const EX: &str = r#"Negate a forward span to get the matching backward span:

```
IMPORT datetime

SUB main()
  LET forward AS Duration = datetime::duration(90)
  LET backward AS Duration = datetime::negate(forward)
END SUB
```

Negation re-normalizes a sub-second span:

```
IMPORT datetime

SUB main()
  LET quarter AS Duration = datetime::duration(0, 250_000_000)
  LET back AS Duration = datetime::negate(quarter)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_negate(d AS Duration) AS Duration
  RETURN __datetime_normDuration(-d.seconds, -d.nanos)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "negate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Duration"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "d",
                desc: "",
                aliases: &[],
                ty: super::ParameterType::named("Duration"),
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::named("Duration"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_negate"),
        }],
    });
}
