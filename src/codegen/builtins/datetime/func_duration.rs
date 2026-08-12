//! `datetime::duration` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/duration.md`.

use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str =
    r#"Build a `Duration` span from seconds, nanoseconds, or larger time components."#;
const DESC: &str = r#"`datetime::duration` builds a signed `Duration`, a span of elapsed time with no
anchor on any timeline. The result carries a whole-second count in its `seconds`
field and a sub-second remainder in its `nanos` field, normalized into the range
`0 .. 999_999_999`. A `Duration` measures a length of time rather than a point in
time; to name a point on the UTC timeline use `datetime::instant` instead.

`duration` is overloaded by argument count, with five disjoint forms selected by
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
argument may be negative, which yields a negative span pointing backward in time.
The one-argument form performs no normalization because its `nanos` is fixed at
zero.

`duration` is overloaded, so every parameter of the form you call must be supplied
explicitly; the component forms carry no defaults.
 The folding and
normalization are ordinary signed `Integer` arithmetic, so a sufficiently large
day, hour, minute, or second magnitude can overflow the `Integer` range and trap.
Combine durations with `datetime::plus`, `datetime::minus`, and `datetime::negate`;
apply one to an `Instant` with `datetime::add` or `datetime::subtract`. `duration`
is pure: the same arguments always yield the same `Duration`, and it has no side
effects."#;
const EX: &str = r#"Build a `Duration` from a whole-second span:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(90)
END SUB
```

Build a `Duration` with a sub-second adjustment that normalizes into the `seconds`
field:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(10, 1_500_000_000)
END SUB
```

Build a `Duration` from day, hour, minute, second, and nanosecond components:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(1, 2, 3, 4, 0)
END SUB
```

A negative argument yields a backward span:

```
IMPORT datetime

SUB main()
  LET d AS Duration = datetime::duration(-30)
END SUB
```"#;

pub(crate) const DURATION: BuiltinFunction = BuiltinFunction::custom(
    "datetime.duration",
    "duration",
    INTRO,
    DESC,
    &[],
    super::DT_COMPONENTS,
)
.with_example(EX);
