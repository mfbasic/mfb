//! `datetime::subtract` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Shift a `datetime::Instant` along the UTC timeline by the opposite of a signed `datetime::Duration`."#;
const DESC: &str = r#"`datetime::subtract` returns the `datetime::Instant` reached by moving `at` along the UTC
timeline by the opposite of the signed span `by`: a positive span moves it earlier,
a negative span later. It subtracts the `seconds` field of `by` from
the `seconds` field of `at` and the `nanos` field of `by` from the `nanos` field
of `at`, independently, then normalizes the difference so the stored `nanos`
lands in the range `0 .. 999_999_999`, borrowing a whole second from the
`seconds` field when the nanosecond difference is negative. The result is a point
on the same Unix-epoch, leap-second-free timeline as `at`.


Because `by` is a signed `datetime::Duration`, `subtract` covers both directions on the
timeline: a positive span moves the `datetime::Instant` earlier and a negative span moves
it later, so `datetime::add(at, by)` and `datetime::subtract(at, by)` name
opposite shifts. The arithmetic is uniform second-and-nanosecond subtraction with
no awareness of calendars, time zones, or daylight-saving transitions; it simply
counts elapsed physical time. For civil, zone-aware day and month arithmetic that
honors DST and varying month lengths, use `datetime::addDays` and
`datetime::addMonths` on a `datetime::DateTime` instead.

Normalization floor-divides the nanosecond difference into a whole-second borrow
and a non-negative remainder, then folds the borrow back into the `seconds`
field, so a subtraction that borrows across the second boundary still yields a
`nanos` in `0 .. 999_999_999`.
The subtraction is ordinary signed `Integer` arithmetic, so a span large enough
to push the combined second count past the `Integer` range raises `ErrOverflow`.
`subtract` is pure: the same `datetime::Instant` and `datetime::Duration` always give the
same result or the same error, and it has no side effects."#;
const EX: &str = r#"Move a `datetime::Instant` back by a 90-second span:

```
IMPORT datetime

SUB main()
  LET base AS datetime::Instant = datetime::instant(1_700_000_000)
  LET earlier AS datetime::Instant = datetime::subtract(base, datetime::duration(90))
END SUB
```

A negative `datetime::Duration` shifts the `datetime::Instant` forward:

```
IMPORT datetime

SUB main()
  LET base AS datetime::Instant = datetime::instant(1_700_000_000)
  LET later AS datetime::Instant = datetime::subtract(base, datetime::duration(-3600))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_subtract(at AS Instant, by AS Duration) AS Instant
  RETURN __datetime_normInstant(at.seconds - by.seconds, at.nanos - by.nanos)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "subtract",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Instant, Duration"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "at",
                    desc: "The instant to shift back from. Not modified.",
                    aliases: &[],
                    ty: super::ParameterType::named("Instant"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "by",
                    desc: "How far back to shift; a negative span moves later. Equivalent to `datetime::add` with the duration negated, except at the most negative seconds count, which `datetime::negate` cannot negate but `subtract` accepts.",
                    aliases: &[],
                    ty: super::ParameterType::named("Duration"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("Instant"),
            errors: vec!["ErrOverflow"],
            body: super::Body::mfb(BODY, "__datetime_subtract"),
        }],
    });
}
