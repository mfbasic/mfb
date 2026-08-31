//! `datetime::minus` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str =
    r#"Subtract one `Duration` span from another and return the resulting `Duration`."#;
const DESC: &str = r#"`datetime::minus` returns the `Duration` `a - b`, the signed span left after
removing one span of elapsed physical time from another. It subtracts the
`seconds` field of `b` from the `seconds` field of `a` and the `nanos` field of
`b` from the `nanos` field of `a`, independently, then normalizes the result so
the stored `nanos` lands in the range `0 .. 999_999_999`, borrowing a whole
second from the `seconds` field when the nanosecond difference is negative.


Because both operands are signed `Duration`s, `minus` handles spans of either
direction: subtracting a negative `Duration` lengthens the total, and
subtracting a larger span from a smaller one yields a negative `Duration`.
`minus` pairs with `datetime::plus` and `datetime::negate`, since
`datetime::minus(a, b)` equals `datetime::plus(a, datetime::negate(b))`. A
common use is measuring elapsed time between two `datetime::monotonic` readings.

Normalization floor-divides the nanosecond difference into a whole-second borrow
and a non-negative remainder, then folds the borrow back into the `seconds`
field, so a `nanos` difference that goes negative still yields a `nanos` in
`0 .. 999_999_999`.
The arithmetic is uniform second-and-nanosecond subtraction with no awareness of
calendars, time zones, or daylight-saving transitions; it simply differences
elapsed physical time. To shift a point on the timeline rather than combine two
spans, use `datetime::subtract` on an `Instant`. The subtraction is ordinary
signed `Integer` arithmetic, so a difference whose second count falls outside the
`Integer` range overflows and traps. `minus` is pure: the same two `Duration`s
always yield the same `Duration`, and it has no side effects."#;
const EX: &str = r#"Subtract a 500-millisecond span from a 90-second span:

```
IMPORT datetime

SUB main()
  LET a AS Duration = datetime::duration(90)
  LET b AS Duration = datetime::duration(0, 500_000_000)
  LET rest AS Duration = datetime::minus(a, b)
END SUB
```

Measure the elapsed time between two monotonic readings:

```
IMPORT datetime

SUB main()
  LET start AS Duration = datetime::monotonic()
  LET finish AS Duration = datetime::monotonic()
  LET elapsed AS Duration = datetime::minus(finish, start)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_minus(a AS Duration, b AS Duration) AS Duration
  RETURN __datetime_normDuration(a.seconds - b.seconds, a.nanos - b.nanos)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "minus",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Duration, Duration"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "a",
                    desc: "The duration to subtract from.",
                    aliases: &[],
                    ty: super::ParameterType::named("Duration"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "b",
                    desc: "The duration to take away. The result is negative when `b` is the longer one.",
                    aliases: &[],
                    ty: super::ParameterType::named("Duration"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("Duration"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_minus"),
        }],
    });
}
