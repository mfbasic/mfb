//! `datetime::addMonths` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

const INTRO: &str = r#"Shift a civil `DateTime` by a whole number of calendar months, clamping the day-of-month to the target month's length."#;
const DESC: &str = r#"`datetime::addMonths` advances `dt` by a whole number of calendar months and
returns the resulting `DateTime`. It collapses `dt`'s year and month into a
single month index (`year * 12 + month - 1`), adds `months`, and splits the sum
back into a target year and month with a flooring divide so that crossing year
boundaries in either direction is handled correctly.
 The wall-clock time of day
and the zone are taken unchanged from `dt`, and the result is re-resolved through
`dt`'s zone so the UTC offset is recomputed for the new date.


Because months vary in length, the day of month is clamped to the number of days
in the target month. If `dt`'s day-of-month exceeds the target month's length the
result lands on the last day of that month, so January 31 plus one month is
February 28 (or February 29 in a leap year), and any earlier day is preserved
exactly. The day is never carried over into the following month.


`months` is a signed count: a positive value moves `dt` later in the calendar and
a negative value moves it earlier; adding zero months returns a `DateTime` with
the same date as `dt`. The operation works purely in whole months and never
alters the hour, minute, second, or nanosecond fields; the sub-second nanosecond
component is carried through unchanged. Because the result is re-resolved through
`dt`'s zone, `addMonths` is daylight-saving aware: the wall-clock time is
preserved while the underlying instant absorbs any offset change for the new
date. For whole-day shifts use `datetime::addDays`, and for uniform physical-time
arithmetic on an `Instant` use `datetime::add`. `addMonths` is pure: the same
`DateTime` and month count always yield the same result, and it has no side
effects."#;
const EX: &str = r#"Advance a `DateTime` by one month:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET nextMonth AS DateTime = datetime::addMonths(dt, 1)
END SUB
```

A negative count moves the date earlier, and an overlong day clamps to the end of
the shorter month:

```
IMPORT datetime

SUB main()
  LET jan31 AS DateTime = datetime::civil(datetime::date(2025, 1, 31), datetime::time(9, 0, 0), datetime::utc())
  LET feb28 AS DateTime = datetime::addMonths(jan31, 1)
  LET lastYear AS DateTime = datetime::addMonths(jan31, -12)
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
  RETURN __datetime_civil(Date[y, m, day], dt.time, dt.zone)
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
                    desc: "How many months to add. Negative subtracts. A day that does not exist in the target month is clamped to that month's last day — 31 January plus one month is 28 or 29 February, not 3 March.",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::named("DateTime"),
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_addMonths"),
        }],
    });
}
