//! `datetime::now` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"The current wall-clock instant on the UTC timeline."#;
const DESC: &str = r#"`datetime::now` reads the host's real-time clock and returns the `Instant` it
names on the UTC timeline (the Unix epoch, without leap seconds). The result
carries whole seconds since `1970-01-01T00:00:00Z` in its `seconds` field and a
sub-second `nanos` field in the range `0 .. 999_999_999`. `now` is the package's
wall-clock entry point that returns an `Instant` (`datetime::nowNanos` reads the
same clock as a bare nanosecond count); project the result through a zone with
`datetime::toUtc`, `datetime::toLocal`, or `datetime::inZone` to obtain civil
fields (year, month, day, and so on).

`now` is `datetime::nowNanos` split into the `seconds` and `nanos` fields of an
`Instant`. The split never fails, and `nanos` always falls in `0 .. 999_999_999`.

`now` is bounded by that nanosecond count, which is valid through roughly the
year 2262. This is a limit on `now`,
not on `Instant`, whose `seconds` field spans the full `Integer` range.

`now` is one of the few `datetime` functions that is **not pure**: two calls may
return different instants, and a program's output depends on the host clock. For
reproducible logic, capture a single instant and derive everything else from it.
`now` takes no arguments, reads host clock state only, and has no side effects."#;
const EX: &str = r#"Capture the current instant:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
END SUB
```

Project the current instant into the local zone to read civil fields:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::now()
  LET here AS DateTime = datetime::toLocal(t)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_now AS Instant
  LET ns AS Integer = datetime::nowNanos()
  RETURN __datetime_normInstant(ns / 1000000000, ns MOD 1000000000)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "now",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::named("Instant"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_now"),
        }],
    });
}
