//! `datetime::toNanos` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/toNanos.md`.

const INTRO: &str = r#"Return the whole nanoseconds between the Unix epoch and an `Instant`."#;
const DESC: &str = r#"`datetime::toNanos` collapses the absolute point `at` into a single `Integer`
count of whole nanoseconds measured from the Unix epoch
(`1970-01-01T00:00:00Z`). Instants before the epoch yield negative counts, the
epoch itself yields `0`, and instants after the epoch yield positive counts.


The result is computed as `at.seconds * 1000000000 + at.nanos`: the
seconds-since-epoch field is scaled to nanoseconds and the sub-second `nanos`
field is added in directly. Because a normalized `Instant` already holds its
`nanos` field at full nanosecond resolution (`0..999999999`), the conversion is
exact and discards nothing — no truncation or rounding occurs in either
direction.

The arithmetic is checked. For an instant near the extreme edge of the timeline
either the `at.seconds * 1000000000` scaling or the trailing addition of
`at.nanos` can exceed the signed `Integer` range, in which case the function
raises `ErrOverflow` rather than wrapping. The range of
representable instants is therefore narrower than for `datetime::toMillis`, since
each second consumes a billion units rather than a thousand.
`datetime::toNanos` is pure: it reads no host state and depends only on `at`.


Unlike `datetime::toMillis`, `datetime::toNanos` preserves the full sub-second
precision of `at`; use it when nanosecond fidelity matters."#;
const EX: &str = r#"Epoch nanoseconds of the current instant:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::toNanos(datetime::now())
END SUB
```

Compare two instants at nanosecond resolution:

```
IMPORT datetime

SUB main()
  LET a AS Integer = datetime::toNanos(datetime::now())
  LET b AS Integer = datetime::toNanos(datetime::now())
  LET elapsed AS Integer = b - a
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_toNanos(at AS Instant) AS Integer
  RETURN at.seconds * 1000000000 + at.nanos
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    super::single(
        pkg,
        "toNanos",
        INTRO,
        DESC,
        EX,
        vec![super::req("at", super::named("Instant"))],
        super::int(),
        BODY,
        "__datetime_toNanos",
    );
}
