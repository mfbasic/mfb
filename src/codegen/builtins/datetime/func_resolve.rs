//! `datetime::resolve` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/resolve.md`.

const INTRO: &str = r#"Collapse a civil `DateTime` back to the absolute `Instant` it names."#;
const DESC: &str = r#"`datetime::resolve` is the inverse of `datetime::inZone`: where `inZone` projects
an absolute instant onto the wall-clock fields an observer in a zone reads,
`resolve` collapses those wall-clock fields — together with the UTC offset already
pinned on `dt` — back onto the single point on the UTC timeline they denote.

The computation is total and needs no zone lookup. `resolve` first converts the
civil date (`dt.date.year`, `dt.date.month`, `dt.date.day`) to a day count with
the proleptic Gregorian calendar, multiplies by `86400` to get seconds, and adds
the time-of-day contribution (`dt.time.hour * 3600 + dt.time.minute * 60 +
dt.time.second`). That sum is the local second count: the seconds-since-epoch the
wall-clock fields would name if they were UTC. It then subtracts `dt.offset` — the
resolved UTC offset in seconds carried on the `DateTime` — to shift the local
count back onto the UTC timeline, and pairs the result with `dt.time.nanos`.


Because the offset is read directly from `dt` rather than re-derived from the
zone, `resolve` is unambiguous even across daylight-saving transitions: it
reproduces exactly the instant a `DateTime` was built from. For any instant `at`
and zone `z`, `datetime::resolve(datetime::inZone(at, z))` returns `at` unchanged.
The `seconds` field participates in the date/time arithmetic; the `nanos` field is
copied through verbatim. `resolve` is pure and reads no host state."#;
const EX: &str = r#"Round-trip an instant through a civil `DateTime` and back:

```
IMPORT datetime

SUB main()
  LET at AS Instant = datetime::now()
  LET dt AS DateTime = datetime::inZone(at, datetime::utc())
  LET back AS Instant = datetime::resolve(dt)
END SUB
```

Resolve a civil `DateTime` built in a fixed +05:30 zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(5, 30)
  LET dt AS DateTime = datetime::inZone(datetime::now(), z)
  LET at AS Instant = datetime::resolve(dt)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_resolve(dt AS DateTime) AS Instant
  LET localSeconds AS Integer = __datetime_daysFromCivil(dt.date.year, dt.date.month, dt.date.day) * 86400 + dt.time.hour * 3600 + dt.time.minute * 60 + dt.time.second
  RETURN Instant[localSeconds - dt.offset, dt.time.nanos]
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    super::single(
        pkg,
        "resolve",
        INTRO,
        DESC,
        EX,
        vec![super::req("dt", super::named("DateTime"))],
        super::named("Instant"),
        BODY,
        "__datetime_resolve",
    );
}
