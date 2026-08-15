//! `datetime::startOfDay` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/startOfDay.md`.

const INTRO: &str = r#"Return the civil `DateTime` naming midnight at the start of a `DateTime`'s day, in its own zone."#;
const DESC: &str = r#"`datetime::startOfDay` returns the `DateTime` naming `00:00:00` (midnight) at the
beginning of `dt`'s civil day, in `dt`'s own zone. It keeps `dt`'s calendar date
(year, month, day) and zone, replaces the wall-clock time with a `Time` of
`00:00:00` and zero nanoseconds, and re-resolves the moment through that zone.


The result is produced exactly as `datetime::civil(dt.date, Time[0, 0, 0, 0],
dt.zone)`: local midnight is interpreted in `dt`'s zone, the applicable UTC offset
is resolved for that moment, and the canonical `DateTime` naming the resulting
`Instant` is returned. Because the offset is re-resolved rather than copied from
`dt`, the result is daylight-saving correct: for the host's local zone the offset
reflects whatever DST rule applies at midnight on that date, which may differ from
the offset that applied at `dt`'s original time of day.


The day boundary is civil midnight in `dt`'s zone, not UTC midnight, so the
underlying `Instant` generally differs from `dt`'s `Instant` truncated to whole
days. Any sub-second nanoseconds carried by `dt` are dropped: the start of the day
has zero nanos. Like `datetime::civil`, the result round-trips through
`datetime::resolve` and `datetime::inZone`.

`datetime::startOfDay` is pure when `dt`'s zone is a fixed-offset zone
(`datetime::utc`, `datetime::fixedOffset`). When `dt`'s zone is the host's local
zone (`datetime::local`), the offset is resolved from the platform's zone table,
so the same `dt` can yield a different absolute instant on a host configured for a
different zone or DST rule."#;
const EX: &str = r#"Truncate a `DateTime` to the start of its civil day:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toLocal(datetime::now())
  LET midnight AS DateTime = datetime::startOfDay(dt)
END SUB
```

Start of day in a fixed UTC zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::utc())
  LET midnight AS DateTime = datetime::startOfDay(dt)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_startOfDay(dt AS DateTime) AS DateTime
  RETURN __datetime_civil(dt.date, Time[0, 0, 0, 0], dt.zone)
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    super::single(
        pkg,
        "startOfDay",
        INTRO,
        DESC,
        EX,
        vec![super::req("dt", super::named("DateTime"))],
        super::named("DateTime"),
        BODY,
        "__datetime_startOfDay",
    );
}
