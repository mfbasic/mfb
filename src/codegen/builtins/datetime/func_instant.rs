//! `datetime::instant` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/instant.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Build an `Instant` from seconds, nanoseconds, or larger time components."#;
const DESC: &str = r#"`datetime::instant` builds an `Instant` on the UTC timeline (the Unix epoch,
without leap seconds) at a given offset after `1970-01-01T00:00:00Z`. The result
carries whole seconds since the epoch in its `seconds` field and a sub-second
remainder in its `nanos` field, normalized into the range `0 .. 999_999_999`.

`instant` is overloaded by argument count, with five disjoint forms selected by
the number of `Integer` arguments (one through five).
 The one- and two-argument forms take
whole seconds and, optionally, a nanosecond adjustment. The three-, four-, and
five-argument forms are component builders that fold larger units down into a
single second count: the three-argument form computes `mins*60 + seconds`, the
four-argument form adds `hours*3600`, and the five-argument form adds
`days*86400`, in every case adding the trailing `nanos` last.


Whichever form is used (except the one-argument form), the supplied seconds and
nanos are normalized: any whole seconds embedded in `nanos` are carried into the
`seconds` field, and a negative `nanos` value borrows a second so the stored
`nanos` always lands in `0 .. 999_999_999`.
 Every numeric
argument may be negative, which selects an instant before the epoch. The
one-argument form performs no normalization because its `nanos` is fixed at zero.


`instant` is overloaded, so every parameter of the form you call must be supplied
explicitly; the component forms carry no defaults.
 The folding and
normalization are ordinary signed `Integer` arithmetic, so a sufficiently large
day, hour, minute, or second magnitude can overflow the `Integer` range and trap.
To shift an existing `Instant` by a span rather than build one from scratch, use
`datetime::add` or `datetime::subtract` with a `Duration`. `instant` is pure: the
same arguments always yield the same `Instant`, and it has no side effects."#;
const EX: &str = r#"Build an `Instant` from a whole-second epoch offset:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::instant(1_700_000_000)
END SUB
```

Build an `Instant` with a sub-second adjustment that normalizes into the `seconds`
field:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::instant(10, 1_500_000_000)
END SUB
```

Build an `Instant` from day, hour, minute, second, and nanosecond components:

```
IMPORT datetime

SUB main()
  LET t AS Instant = datetime::instant(1, 2, 3, 4, 0)
END SUB
```"#;

pub(crate) const INSTANT: BuiltinFunction = BuiltinFunction::custom(
    "datetime.instant",
    "instant",
    INTRO,
    DESC,
    &[],
    super::INSTANT_OVERLOADS,
)
.with_example(EX);
