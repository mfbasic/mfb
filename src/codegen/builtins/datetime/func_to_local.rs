//! `datetime::toLocal` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/toLocal.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str =
    r#"Project an absolute `Instant` into the host's local zone to produce a civil `DateTime`."#;
const DESC: &str = r#"`datetime::toLocal` projects the absolute instant `at` into the host's local
time zone, yielding the calendar date and wall-clock time that an observer
reading the local clock sees at that moment. It is exactly shorthand for
`datetime::inZone(at, datetime::local())`: it resolves the host's effective UTC
offset for the instant `at` (see `datetime::offsetAt`), with daylight-saving
time applied as it stood at that instant, adds that offset in seconds to the
instant's seconds-since-epoch to obtain a local second count, floor-divides that
into whole days and the second-of-day, converts the day count to a civil
year/month/day with the proleptic Gregorian calendar, and decomposes the
second-of-day into hour, minute, and second.



The returned `DateTime` carries four things: the civil date, the civil time, the
local zone, and the resolved offset. Because the resolved offset is pinned onto
the result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second
`nanos` field is preserved verbatim into the time's `nanos` field; only the
`seconds` field participates in the offset and date/time computation, so an
instant before the Unix epoch (negative `seconds`) projects correctly.


Unlike `datetime::toUtc`, `datetime::toLocal` is not pure: it reads the host's
time-zone configuration to resolve the offset, so the same instant can produce a
different civil `DateTime` on a host configured for a different zone or under a
different DST rule."#;
const EX: &str = r#"Project the current instant into the host's local zone:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toLocal(datetime::now())
END SUB
```

Round-trip an instant through the local zone and back:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET dt AS DateTime = datetime::toLocal(at)
  LET back AS Instant = datetime::resolve(dt)
END SUB
```"#;

pub(crate) const TO_LOCAL: BuiltinFunction = BuiltinFunction::custom(
    "datetime.toLocal",
    "toLocal",
    INTRO,
    DESC,
    &[],
    &[super::ov(&[super::req("at", "Instant")], "DateTime")],
)
.with_example(EX);
