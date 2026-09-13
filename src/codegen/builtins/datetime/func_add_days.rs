//! `datetime::addDays` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Shift a civil `datetime::DateTime` by a whole number of calendar days, preserving its wall-clock time and zone."#;
const DESC: &str = r#"`datetime::addDays` advances `dt` by a whole number of calendar days and returns
the resulting `datetime::DateTime`. It converts `dt`'s calendar date to a serial day count,
adds `days`, converts that count back to a year-month-day date, and rebuilds the
`datetime::DateTime` from the new date, `dt`'s original wall-clock time, and `dt`'s original
zone.

For a `datetime::local` zone `addDays` is daylight-saving aware: the wall-clock
time of day is preserved and the UTC offset is worked out for the new date, so
crossing a DST transition shifts the underlying instant by the appropriate 23-,
24-, or 25-hour day rather than a fixed `86_400` seconds. `dt`'s offset is kept
whenever it is still valid at the new date and time, and only otherwise
re-resolved the way `datetime::civil` resolves a local time. So when the result
lands in a fall-back overlap, where the wall-clock time happens twice, it stays
on the side `dt`'s offset names if that offset is one of the two. A fixed-offset
or UTC zone keeps its offset. The sub-second nanosecond component of the time is
carried through unchanged.

`days` is a signed count: a positive value moves `dt` later in the calendar and a
negative value moves it earlier. Adding zero days returns a `datetime::DateTime` equal to
`dt`. The operation works purely in whole days and never alters the hour, minute,
second, or nanosecond fields; for month-length-aware shifts use
`datetime::addMonths`, and for uniform physical-time arithmetic on a `datetime::Instant`
use `datetime::add`. `addDays` has no side effects. For a UTC or fixed-offset zone
the same `datetime::DateTime` and day count always yield the same result. For a
`datetime::local` zone the offset comes from the host's time-zone rules, so the
same `dt` can yield a different absolute instant on a host configured for a
different zone or DST rule."#;
const EX: &str = r#"Advance a `datetime::DateTime` by one week:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::toUtc(datetime::now())
  LET nextWeek AS datetime::DateTime = datetime::addDays(dt, 7)
END SUB
```

A negative count moves the date earlier:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::toUtc(datetime::now())
  LET yesterday AS datetime::DateTime = datetime::addDays(dt, -1)
END SUB
```

Cross a daylight-saving transition. Both examples above use `datetime::utc()`
values, which have no transitions, so neither reaches the re-resolution this
member's description promises — this one does. It assumes `TZ=America/New_York`,
because `datetime::local()` reads the host zone and there is no way to name a
zone in the language yet. The values shown are measured:

```
IMPORT io
IMPORT datetime

SUB main()
  LET before AS datetime::DateTime = datetime::civil(datetime::date(2026, 3, 7), datetime::time(12, 0), datetime::local())
  LET after AS datetime::DateTime = datetime::addDays(before, 1)

  io::print(datetime::format(before, "yyyy-MM-dd HH:mm:ss ZZ"))
  ' 2026-03-07 12:00:00 -05:00
  io::print(datetime::format(after, "yyyy-MM-dd HH:mm:ss ZZ"))
  ' 2026-03-08 12:00:00 -04:00

  ' The WALL CLOCK is preserved — both read 12:00 — while the offset moved from
  ' -05:00 to -04:00. So the underlying instant advanced by 23 hours, not 24:
  ' this prints 82800, not 86400.
  LET moved AS Integer = datetime::resolve(after).seconds - datetime::resolve(before).seconds
  io::print(toString(moved))
  ' 82800
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_addDays(dt AS DateTime, days AS Integer) AS DateTime
  LET newDays AS Integer = __datetime_daysFromCivil(dt.date.year, dt.date.month, dt.date.day) + days
  ' plan-64 A3: fixed-offset fast path. A whole-day shift leaves the wall-clock
  ' time and the zone offset unchanged, and civilFromDays/daysFromCivil are
  ' inverse on valid civil dates, so for a fixed-offset zone (kind <> 2)
  ' __datetime_civil's resolveLocal->Instant->inZone round-trip provably returns
  ' DateTime[civilFromDays(newDays), dt.time, dt.zone, dt.offset]. Build it
  ' directly to skip that round-trip's Instant/Date/Time transient allocations.
  ' A system zone (kind = 2) can cross a DST boundary, so it keeps the round-trip.
  IF dt.zone.kind <> 2 THEN
    RETURN DateTime[__datetime_civilFromDays(newDays), dt.time, dt.zone, dt.offset]
  END IF
  RETURN __datetime_civilKeepOffset(__datetime_civilFromDays(newDays), dt.time, dt.zone, dt.offset)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "addDays",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("DateTime, Integer"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "dt",
                    desc: "The date-time to shift. Not modified.",
                    aliases: &[],
                    ty: super::ParameterType::named("DateTime"),
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "days",
                    desc: "How many days to add. Negative subtracts. Calendar days, so a day that a zone transition shortens or lengthens still counts as one.",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_addDays"),
        }],
    });
}
