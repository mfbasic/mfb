//! `datetime::civil` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/civil.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Build a zoned `DateTime` from a civil `Date`, `Time`, and `Zone`."#;
const DESC: &str = r#"`datetime::civil` builds a `DateTime` by reading a calendar `date` and a
wall-clock `time` as a local time in `zone`, resolving the UTC offset that
applies to that local moment, and returning the canonical projection of the
resulting `Instant` back through `zone`. Because the result is the projection of
a concrete `Instant`, it round-trips: `datetime::resolve` on the returned
`DateTime` recovers the same `Instant`, and that `Instant` projected through
`zone` with `datetime::inZone` reproduces the same `DateTime` fields.


The `year`, `month`, and `day` of `date` and the `hour`, `minute`, and `second`
of `time` are combined into a single second count (`daysFromCivil * 86400 +
hour * 3600 + minute * 60 + second`) that names the wall-clock moment, treated
as a civil (zone-local) time. The offset for that moment is then resolved from
`zone`. For a zone with a fixed offset (built by `datetime::utc` or
`datetime::fixedOffset`) the offset is constant; for the host's local zone
(`datetime::local`) it is resolved from the platform's zone table at that
instant, so the result is daylight-saving correct.


When the named local time does not exist or is not unique because of a
daylight-saving transition, `civil` resolves it deterministically. It probes the
zone's offset one day before and one day after the named local time to bracket
any single nearby transition. If both probes agree, that offset is used
directly. If they differ, a spring-forward gap (the named local time is skipped)
shifts forward onto the post-transition offset, and a fall-back overlap (the
named local time occurs twice) takes the earlier, pre-transition offset.


The sub-second `nanos` of `time` are carried through unchanged into the
resulting `Instant` and `DateTime`; only the whole-second civil fields
participate in offset resolution. `civil` is pure: beyond what `zone` itself
resolves it reads no host state and has no side effects."#;
const EX: &str = r#"Combine a date and time into a `DateTime` in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
END SUB
```

Build a `DateTime` in UTC and recover its `Instant`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 1, 1)
  LET tm AS Time = datetime::time(0, 0)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
  LET at AS Instant = datetime::resolve(dt)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_civil(d AS Date, t AS Time, z AS Zone) AS DateTime
  LET localSeconds AS Integer = __datetime_daysFromCivil(d.year, d.month, d.day) * 86400 + t.hour * 3600 + t.minute * 60 + t.second
  LET epochSeconds AS Integer = __datetime_resolveLocal(localSeconds, z)
  RETURN __datetime_inZone(Instant[epochSeconds, t.nanos], z)
END FUNC"#;

pub(crate) const CIVIL: BuiltinFunction = BuiltinFunction::mfb(
    "datetime.civil",
    "civil",
    INTRO,
    DESC,
    &[],
    &[super::ov(
        &[
            super::req("date", "Date"),
            super::req("time", "Time"),
            super::req("zone", "Zone"),
        ],
        "DateTime",
    )],
    BODY,
)
.with_example(EX);
