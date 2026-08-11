//! `datetime::inZone` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/inZone.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Project an absolute `Instant` into a `Zone` to produce a civil `DateTime`."#;
const DESC: &str = r#"`datetime::inZone` is the primary "to civil time" call: it projects the absolute
instant `at` through `zone`, yielding the calendar date and wall-clock time that
an observer in that zone reads at that moment.

It first resolves the effective UTC offset for `zone` at the instant `at` — the
same quantity `datetime::offsetAt` returns: zero for a UTC zone
(`ZoneKind::Utc`), the stored constant for a fixed-offset zone (`ZoneKind::FixedOffset`,
kind `1`, built with `datetime::fixedOffset`), and the DST-correct host offset
for a local zone (`ZoneKind::Local`, kind `2`, built with `datetime::local`).
 It then adds
that offset, in seconds, to the instant's seconds-since-epoch to obtain a local
second count, floor-divides that into whole days and the second-of-day, converts
the day count to a civil year/month/day with the proleptic Gregorian calendar,
and decomposes the second-of-day into hour, minute, and second.


The returned `DateTime` carries four things: the civil date, the civil time,
`zone` itself, and the resolved offset. Because the offset is pinned onto the
result, the `DateTime` round-trips back to the original instant via
`datetime::resolve` with no further zone lookup. The instant's sub-second `nanos`
field is preserved verbatim into the time's `nanos` field; only the `seconds`
field participates in the offset and date/time computation, so an instant before
the Unix epoch (negative `seconds`) projects correctly.


`datetime::toUtc` and `datetime::toLocal` are shorthands for calling `inZone`
with the UTC zone and the host local zone, respectively. `inZone` is pure for UTC
and fixed-offset zones; for a local zone it reads the host's time-zone
configuration through the `datetime::localOffset` OS intrinsic to resolve the
offset."#;
const EX: &str = r#"Project the current instant into UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::utc())
END SUB
```

Project an instant into a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET dt AS DateTime = datetime::inZone(datetime::now(), z)
END SUB
```

Project into the host's local zone, with DST applied for that instant:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::inZone(datetime::now(), datetime::local())
END SUB
```"#;

pub(crate) const IN_ZONE: BuiltinFunction = BuiltinFunction::custom(
    "datetime.inZone",
    "inZone",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[super::req("at", "Instant"), super::req("zone", "Zone")],
        "DateTime",
    )],
)
.with_example(EX);
