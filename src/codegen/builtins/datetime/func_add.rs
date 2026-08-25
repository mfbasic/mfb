//! `datetime::add` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Shift an `Instant` forward along the UTC timeline by a `Duration`."#;
const DESC: &str = r#"`datetime::add` returns the `Instant` reached by advancing `at` forward along
the UTC timeline by the span `by`. It adds the two `seconds` fields and the two
`nanos` fields independently, then normalizes the sum so the stored `nanos`
lands in the range `0 .. 999_999_999`, carrying any whole seconds embedded in
the nanosecond sum into the `seconds` field. The result is a point on the same
Unix-epoch, leap-second-free timeline as `at`.


Because `by` is a signed `Duration`, `add` covers both directions on the
timeline: a positive span moves the `Instant` later and a negative span moves it
earlier, so `datetime::add(at, by)` and `datetime::subtract(at, by)` name
opposite shifts. The arithmetic is uniform second-and-nanosecond addition with
no awareness of calendars, time zones, or daylight-saving transitions; it simply
counts elapsed physical time. For civil, zone-aware day and month arithmetic
that honors DST and varying month lengths, use `datetime::addDays` and
`datetime::addMonths` on a `DateTime` instead.

Normalization floor-divides the nanosecond sum into a whole-second carry and a
non-negative remainder, then folds the carry back into the `seconds` field, so
a negative `Duration` that borrows across the second boundary still yields a
`nanos` in `0 .. 999_999_999`.
The addition is ordinary signed `Integer` arithmetic, so a span large enough to
push the combined second count past the `Integer` range overflows and traps.
`add` is pure: the same `Instant` and `Duration` always yield the same
`Instant`, and it has no side effects."#;
const EX: &str = r#"Advance an `Instant` by a 90-second span:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET later AS Instant = datetime::add(base, datetime::duration(90))
END SUB
```

A negative `Duration` shifts the `Instant` backward:

```
IMPORT datetime

SUB main()
  LET base AS Instant = datetime::instant(1_700_000_000)
  LET earlier AS Instant = datetime::add(base, datetime::duration(-3600))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_add(at AS Instant, by AS Duration) AS Instant
  RETURN __datetime_normInstant(at.seconds + by.seconds, at.nanos + by.nanos)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "add",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Instant, Duration"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "at",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::named("Instant"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "by",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::named("Duration"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("Instant"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_add"),
        }],
    });
}
