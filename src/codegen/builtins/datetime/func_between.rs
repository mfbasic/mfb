//! `datetime::between` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"The signed `datetime::Duration` span between two instants."#;
const DESC: &str = r#"`datetime::between` returns the signed `datetime::Duration` `endTime - start`: the length of
elapsed time you would add to `start` to reach `endTime`. The span is positive when
`endTime` is later than `start`, negative when `endTime` is earlier, and zero when
the two instants are equal. Because the result is a `datetime::Duration` it carries no anchor
on the timeline — it names a length, not a point.

The span is computed by subtracting the two `datetime::Instant`s field by field
(`endTime.seconds - start.seconds` and `endTime.nanos - start.nanos`) and then
normalizing the pair so the stored `nanos` lands in `0 .. 999_999_999` and any
borrow is carried into the `seconds` field. A negative nanosecond difference
borrows a whole second during normalization, so the `seconds` field of the result
is the floored whole-second component of the true difference and the `nanos` field
is the non-negative sub-second remainder.



Both instants are points on the same Unix-epoch, leap-second-free UTC timeline, so
the span is independent of any time zone; resolve a `datetime::DateTime` to a `datetime::Instant` with
`datetime::resolve` before measuring. `between` is pure: the same two instants
always yield the same `datetime::Duration`, and it has no side effects. The subtraction and
the normalizing carry are ordinary signed `Integer` arithmetic, so two instants far
enough apart that their second difference falls outside the signed `Integer` range
overflow and trap. Render the result with `datetime::formatDuration`, and combine or
apply spans with `datetime::plus`, `datetime::minus`, `datetime::negate`,
`datetime::add`, and `datetime::subtract`."#;
const EX: &str = r#"Measure the span between two instants and render it:

```
IMPORT datetime
IMPORT io

SUB main()
  LET start AS datetime::Instant = datetime::instant(1_000)
  LET endTime AS datetime::Instant = datetime::instant(1_090)
  LET span AS datetime::Duration = datetime::between(start, endTime)
  io::print(datetime::formatDuration(span))
END SUB
```

An `endTime` earlier than `start` yields a negative span:

```
IMPORT datetime

SUB main()
  LET start AS datetime::Instant = datetime::instant(1_090)
  LET endTime AS datetime::Instant = datetime::instant(1_000)
  LET span AS datetime::Duration = datetime::between(start, endTime)
END SUB
```

Re-apply the measured span to recover `endTime` from `start`:

```
IMPORT datetime

SUB main()
  LET start AS datetime::Instant = datetime::instant(1_000)
  LET endTime AS datetime::Instant = datetime::instant(1_090)
  LET span AS datetime::Duration = datetime::between(start, endTime)
  LET again AS datetime::Instant = datetime::add(start, span)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_between(start AS Instant, endTime AS Instant) AS Duration
  RETURN __datetime_normDuration(endTime.seconds - start.seconds, endTime.nanos - start.nanos)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "between",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Instant, Instant"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "start",
                    desc: "The earlier instant. If it is actually the later one the result is negative.",
                    aliases: &[],
                    ty: super::ParameterType::named("Instant"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "endTime",
                    desc: "The later instant. The result is `endTime - start`.",
                    aliases: &[],
                    ty: super::ParameterType::named("Instant"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("Duration"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_between"),
        }],
    });
}
