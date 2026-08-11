//! `datetime::now` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/now.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"The current wall-clock instant on the UTC timeline."#;
const DESC: &str = r#"`datetime::now` reads the host's real-time clock and returns the `Instant` it
names on the UTC timeline (the Unix epoch, without leap seconds). The result
carries whole seconds since `1970-01-01T00:00:00Z` in its `seconds` field and a
sub-second `nanos` field in the range `0 .. 999_999_999`. `now` is the only
wall-clock entry point in the package; project the result through a zone with
`datetime::toUtc`, `datetime::toLocal`, or `datetime::inZone` to obtain civil
fields (year, month, day, and so on).

Internally `now` takes a single nanoseconds-since-epoch reading from the OS
intrinsic (`datetime::nowNanos`), then splits it into the `seconds` and `nanos`
fields of an `Instant` by a truncating divide and remainder against
`1_000_000_000`. The reading is non-negative and the divisor is a non-zero
constant, so the split cannot trap, and the nanosecond remainder already falls
in `0 .. 999_999_999`.

`now` is bounded by its underlying intrinsic, which reports nanoseconds since
the epoch and is valid through roughly the year 2262. This is a limit on `now`,
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

pub(crate) const NOW: BuiltinFunction = BuiltinFunction::custom(
    "datetime.now",
    "now",
    INTRO,
    DESC,
    &[],
    &[super::ov(&[], "Instant")],
)
.with_example(EX);
