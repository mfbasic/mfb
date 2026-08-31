//! `datetime::toMillis` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Return the whole milliseconds between the Unix epoch and an `Instant`."#;
const DESC: &str = r#"`datetime::toMillis` collapses the absolute point `at` into a single `Integer`
count of whole milliseconds measured from the Unix epoch
(`1970-01-01T00:00:00Z`). Instants before the epoch yield negative counts, the
epoch itself yields `0`, and instants after the epoch yield positive counts.


The result is computed as `at.seconds * 1000 + at.nanos / 1000000`: the
seconds-since-epoch field is scaled to milliseconds and the sub-second `nanos`
field contributes its whole-millisecond part. The `nanos` division truncates,
discarding any sub-millisecond remainder (the microsecond and nanosecond
digits). Because a normalized `Instant` always holds a non-negative `nanos`
field in the range `0..999999999`, this truncation drops the fractional
millisecond rather than rounding it, in either direction.

The arithmetic is checked. For an instant near the extreme edge of the timeline
either the `at.seconds * 1000` scaling or the following addition can exceed the
signed `Integer` range, in which case the function raises `ErrOverflow` rather
than wrapping. `datetime::toMillis` is pure: it reads no host state and depends
only on `at`.

`datetime::toMillis` is the inverse of `datetime::fromMillis` to
whole-millisecond precision; sub-millisecond `nanos` in `at` are not recoverable
from the result. For full nanosecond precision use `datetime::toNanos`."#;
const EX: &str = r#"Epoch milliseconds of the current instant:

```
IMPORT datetime

SUB main()
  LET ms AS Integer = datetime::toMillis(datetime::now())
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
r#"FUNC __datetime_toMillis(at AS Instant) AS Integer
  RETURN at.seconds * 1000 + at.nanos / 1000000
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "toMillis",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Instant"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "at",
                desc: "The instant to convert. Sub-millisecond precision is discarded.",
                aliases: &[],
                ty: super::ParameterType::named("Instant"),
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::Integer,
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_toMillis"),
        }],
    });
}
