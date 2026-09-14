//! `datetime::addMonths` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Shift a civil `datetime::DateTime` by a whole number of calendar months, clamping the day-of-month to the target month's length."#;
const DESC: &str = r#"`datetime::addMonths` advances `dt` by a whole number of calendar months and
returns the resulting `datetime::DateTime`. Crossing a year boundary works in either
direction: one month after December 15 is January 15 of the next year. The
wall-clock time of day and the zone are taken from `dt`. For a UTC or fixed-offset
zone the result keeps `dt`'s stored offset as it is. For a `datetime::local` zone
the offset is kept when it still applies at the new date and time, and otherwise
the new local time is resolved the way `datetime::civil` resolves it.


Because months vary in length, the day of month is clamped to the number of days
in the target month. If `dt`'s day-of-month exceeds the target month's length the
result lands on the last day of that month, so January 31 plus one month is
February 28 (or February 29 in a leap year), and any earlier day is preserved
exactly. The day is never carried over into the following month.


`months` is a signed count: a positive value moves `dt` later in the calendar and
a negative value moves it earlier; adding zero months returns a `datetime::DateTime`
equal to `dt`. The hour, minute, second, and nanosecond fields are kept, except
when the result falls in a local zone's spring-forward gap: that wall-clock time
does not exist, so it moves forward the way `datetime::civil` resolves a gap
(under `TZ=America/New_York`, 2026-02-08 02:30 -05:00 plus one month is
2026-03-08 03:30 -04:00). For a `datetime::local` zone `addMonths`
is daylight-saving aware: the wall-clock time is preserved while the underlying
instant absorbs any offset change for the new date, and a result in a fall-back
overlap stays on the side `dt`'s offset names when that offset is one of the two.
For whole-day shifts use `datetime::addDays`, and for uniform physical-time
arithmetic on a `datetime::Instant` use `datetime::add`. `addMonths` has no side
effects. For a UTC or fixed-offset zone the same `datetime::DateTime` and month
count always yield the same result. For a `datetime::local` zone the offset comes
from the host's time-zone rules, so the same `dt` can yield a different absolute
instant on a host configured for a different zone or DST rule.

A `months` count that moves the date or its second count past the `Integer`
range raises `ErrOverflow`. For a local zone, a result time the host cannot
convert raises `ErrInvalidArgument`, as `datetime::localOffset` does."#;
const EX: &str = r#"Advance a `datetime::DateTime` by one month:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::toUtc(datetime::now())
  LET nextMonth AS datetime::DateTime = datetime::addMonths(dt, 1)
END SUB
```

A negative count moves the date earlier, and an overlong day clamps to the end of
the shorter month:

```
IMPORT datetime

SUB main()
  LET jan31 AS datetime::DateTime = datetime::civil(datetime::date(2025, 1, 31), datetime::time(9, 0, 0), datetime::utc())
  LET feb28 AS datetime::DateTime = datetime::addMonths(jan31, 1)
  LET lastYear AS datetime::DateTime = datetime::addMonths(jan31, -12)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_addMonths(dt AS DateTime, months AS Integer) AS DateTime
  LET total AS Integer = dt.date.year * 12 + (dt.date.month - 1) + months
  LET y AS Integer = __datetime_floorDiv(total, 12)
  LET m AS Integer = __datetime_floorMod(total, 12) + 1
  MUT day AS Integer = dt.date.day
  LET dim AS Integer = __datetime_daysInMonth(y, m)
  IF day > dim THEN
    day = dim
  END IF
  ' plan-64 A3: fixed-offset fast path (see __datetime_addDays). day is clamped to
  ' daysInMonth so Date[y, m, day] is a valid civil date; for a fixed-offset zone
  ' __datetime_civil would round-trip back to the same fields, so build directly.
  IF dt.zone.kind <> 2 THEN
    RETURN DateTime[Date[y, m, day], dt.time, dt.zone, dt.offset]
  END IF
  RETURN __datetime_civilKeepOffset(Date[y, m, day], dt.time, dt.zone, dt.offset)
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "addMonths",
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
                    name: "months",
                    desc: "How many months to add. Negative subtracts; zero returns a date-time equal to `dt`. A day that does not exist in the target month is clamped to that month's last day — 31 January plus one month is 28 or 29 February, not 3 March.",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec!["ErrInvalidArgument", "ErrOverflow"],
            body: super::Body::mfb(BODY, "__datetime_addMonths"),
        }],
    });
}
