//! `datetime::compare` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/compare.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Order two instants on the UTC timeline as a three-way sign."#;
const DESC: &str = r#"`datetime::compare` returns the sign of `a - b` as a three-way ordering: `-1`
when `a` is before `b`, `0` when the two instants name the same point, and `1`
when `a` is after `b`. The result is the standard comparator value suitable for
driving a sort or a branch on ordering, and it never returns any value other
than `-1`, `0`, or `1`.

The comparison is performed field by field. The `seconds` fields are compared
first: if `a.seconds` is less than `b.seconds` the result is `-1`, and if it is
greater the result is `1`. Only when the `seconds` fields are equal are the
`nanos` fields compared the same way, so the sub-second component acts as a
tiebreaker. When both `seconds` and `nanos` are equal the instants are
identical and the result is `0`. Because both arguments are points on the same
Unix-epoch, leap-second-free UTC timeline, the ordering is absolute and
independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before comparing.

`compare` is pure: the same two instants always yield the same `Integer`, it
has no side effects, and it performs only signed comparisons (no arithmetic),
so it cannot overflow or trap. For a `Boolean` test rather than a three-way
sign, use `datetime::isBefore`, `datetime::isAfter`, or `datetime::equals`, each
of which is defined in terms of `compare`. To measure the size of the gap
between two instants rather than just their order, use `datetime::between`."#;
const EX: &str = r#"Order two instants:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(2_000)
  io::print(toString(datetime::compare(a, b)))
END SUB
```

Equal instants compare as zero:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::compare(a, b)))
END SUB
```

Branch on the three-way ordering:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::now()
  LET b AS Instant = datetime::instant(0)
  LET order AS Integer = datetime::compare(a, b)
  IF order < 0 THEN io::print("a is earlier")
END SUB
```"#;

pub(crate) const COMPARE: BuiltinFunction = BuiltinFunction::custom(
    "datetime.compare",
    "compare",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[super::req("a", "Instant"), super::req("b", "Instant")],
        super::I,
    )],
)
.with_example(EX);
