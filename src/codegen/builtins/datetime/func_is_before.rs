//! `datetime::isBefore` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`.

const INTRO: &str = r#"Test whether one instant strictly precedes another on the UTC timeline."#;
const DESC: &str = r#"`datetime::isBefore` is a convenience predicate over instants that returns
`TRUE` when `a` strictly precedes `b` on the UTC timeline and `FALSE` otherwise.
It is defined directly in terms of `datetime::compare`: the result is exactly
`datetime::compare(a, b) < 0`, so it is `TRUE` only when `compare` reports `-1`
and `FALSE` when `compare` reports `0` or `1`.


The comparison is performed field by field, matching `datetime::compare`. The
`seconds` fields are compared first; only when they are equal are the `nanos`
fields used as a tiebreaker. As a consequence, two instants that name the same
point (equal `seconds` and equal `nanos`) are not "before" each other, so
`isBefore` returns `FALSE` for equal instants — the relation is strict, not
"before or equal". Because both arguments are points on the same Unix-epoch,
leap-second-free UTC timeline, the ordering is absolute and independent of any
time zone; resolve a `DateTime` to an `Instant` with `datetime::resolve` before
comparing.

`isBefore` is pure: the same two instants always yield the same `Boolean`, it
has no side effects, and it performs only signed comparisons (no arithmetic), so
it cannot overflow or trap. For the symmetric test use `datetime::isAfter`, for
an equality test use `datetime::equals`, and for a three-way sign rather than a
`Boolean` use `datetime::compare`. To measure the size of the gap between two
instants rather than just their order, use `datetime::between`."#;
const EX: &str = r#"An earlier instant is before a later one:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(2_000)
  io::print(toString(datetime::isBefore(a, b)))
END SUB
```

Equal instants are not before each other:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::isBefore(a, b)))
END SUB
```

Branch on chronological order:

```
IMPORT datetime
IMPORT io

SUB main()
  LET past AS Instant = datetime::instant(0)
  LET nowInstant AS Instant = datetime::now()
  IF datetime::isBefore(past, nowInstant) THEN io::print("past is earlier")
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_isBefore(a AS Instant, b AS Instant) AS Boolean
  RETURN __datetime_compare(a, b) < 0
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "isBefore",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Instant, Instant"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "a",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Named("Instant"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "b",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Named("Instant"),
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::Boolean,
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_isBefore"),
        }],
    });
}
