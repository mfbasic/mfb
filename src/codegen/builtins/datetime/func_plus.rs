//! `datetime::plus` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`.

const INTRO: &str = r#"Add two `Duration` spans into their combined `Duration`."#;
const DESC: &str = r#"`datetime::plus` returns the `Duration` `a + b`, the signed span that results
from combining two spans of elapsed physical time. It adds the two `seconds`
fields and the two `nanos` fields independently, then normalizes the sum so the
stored `nanos` lands in the range `0 .. 999_999_999`, carrying any whole seconds
embedded in the nanosecond sum into the `seconds` field.


Because both operands are signed `Duration`s, `plus` handles spans of either
direction: adding a negative `Duration` shortens the total, and adding two
`Duration`s of opposite sign moves toward zero. The operation is commutative —
`datetime::plus(a, b)` and `datetime::plus(b, a)` yield the same `Duration` — and
pairs with `datetime::minus` and `datetime::negate`, since
`datetime::plus(a, datetime::negate(b))` equals `datetime::minus(a, b)`.

Normalization floor-divides the nanosecond sum into a whole-second carry and a
non-negative remainder, then folds the carry back into the `seconds` field, so a
combined `nanos` that overflows or goes negative still yields a `nanos` in
`0 .. 999_999_999`.
The arithmetic is uniform second-and-nanosecond addition with no awareness of
calendars, time zones, or daylight-saving transitions; it simply totals elapsed
physical time. To shift a point on the timeline rather than combine two spans,
use `datetime::add` on an `Instant`. The addition is ordinary signed `Integer`
arithmetic, so a combined second count that exceeds the `Integer` range
overflows and traps. `plus` is pure: the same two `Duration`s always yield the
same `Duration`, and it has no side effects."#;
const EX: &str = r#"Combine a 90-second span with a 500-millisecond span:

```
IMPORT datetime

SUB main()
  LET a AS Duration = datetime::duration(90)
  LET b AS Duration = datetime::duration(0, 500_000_000)
  LET total AS Duration = datetime::plus(a, b)
END SUB
```

Adding a negative `Duration` shortens the total:

```
IMPORT datetime

SUB main()
  LET a AS Duration = datetime::duration(3600)
  LET total AS Duration = datetime::plus(a, datetime::duration(-600))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_plus(a AS Duration, b AS Duration) AS Duration
  RETURN __datetime_normDuration(a.seconds + b.seconds, a.nanos + b.nanos)
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "plus",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Duration, Duration"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "a",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Named("Duration"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "b",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Named("Duration"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::Named("Duration"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_plus"),
        }],
    });
}
