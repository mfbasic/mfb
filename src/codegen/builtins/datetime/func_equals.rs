//! `datetime::equals` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/equals.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Test whether two instants name the same point on the UTC timeline."#;
const DESC: &str = r#"`datetime::equals` is a convenience predicate over instants that returns `TRUE`
when `a` and `b` name the same point on the UTC timeline and `FALSE` otherwise.
It is defined directly in terms of `datetime::compare`: the result is exactly
`datetime::compare(a, b) = 0`, so it is `TRUE` only when `compare` reports `0`
and `FALSE` when `compare` reports `-1` or `1`.


The comparison is performed field by field, matching `datetime::compare`. The
`seconds` fields are compared first; only when they are equal are the `nanos`
fields used as a tiebreaker. Two instants are equal only when both their
`seconds` and their `nanos` fields are equal, so equality is exact to the
nanosecond and there is no tolerance window. Because both arguments are points
on the same Unix-epoch, leap-second-free UTC timeline, the test is absolute and
independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before comparing.

`equals` is pure: the same two instants always yield the same `Boolean`, it has
no side effects, and it performs only signed comparisons (no arithmetic), so it
cannot overflow or trap. For the strict ordering tests use `datetime::isBefore`
and `datetime::isAfter`, and for a three-way sign rather than a `Boolean` use
`datetime::compare`. To measure the size of the gap between two instants rather
than just whether they coincide, use `datetime::between`."#;
const EX: &str = r#"Equal instants compare as equal:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::equals(a, b)))
END SUB
```

Different instants are not equal:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(2_000)
  io::print(toString(datetime::equals(a, b)))
END SUB
```

Branch on whether two instants coincide:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::now()
  LET b AS Instant = datetime::instant(0)
  IF datetime::equals(a, b) THEN io::print("same instant")
END SUB
```"#;

pub(crate) const EQUALS: BuiltinFunction = BuiltinFunction::custom(
    "datetime.equals",
    "equals",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[super::req("a", "Instant"), super::req("b", "Instant")],
        "Boolean",
    )],
)
.with_example(EX);
