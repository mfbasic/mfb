//! `datetime::dayOfYear` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/dayOfYear.md`.

const INTRO: &str = r#"The ordinal day within the year of a `DateTime`'s civil date."#;
const DESC: &str = r#"`datetime::dayOfYear` returns the ordinal position of `dt`'s civil date within
its calendar year: `1` for January 1, `2` for January 2, and so on through `365`
in a common year or `366` in a leap year (the value reached on December 31).


The result is derived solely from the calendar date fields carried by `dt` — its
year, month, and day as stored in `dt`'s own zone. The day-of-year is computed on
the proleptic-Gregorian calendar by taking the days-from-civil count of `dt`'s
date, subtracting the days-from-civil count of January 1 of the same year, and
adding one (`here - start + 1`), so leap years correctly extend the count past
February. The time-of-day fields, the sub-second nanoseconds, and the zone's UTC
offset do not affect the result; no `Instant` is resolved and no zone table is
consulted.

Because the computation reads only `dt`'s stored civil date, the same instant
projected into two different zones can report two different day-of-year values
whenever the zones place that instant on opposite sides of midnight, and across
the December 31 / January 1 boundary the two zones can even fall in different
years.

`datetime::dayOfYear` is pure: it reads no host state and has no side effects."#;
const EX: &str = r#"Find the day-of-year of a civil date in the local zone:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET tm AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, tm, datetime::local())
  LET n AS Integer = datetime::dayOfYear(dt)
END SUB
```

Compute how many days remain in the year:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::civil(datetime::date(2026, 6, 26), datetime::time(9, 30), datetime::local())
  MUT total AS Integer = 365
  IF datetime::isLeapYear(dt.date.year) THEN
    total = 366
  END IF
  LET remaining AS Integer = total - datetime::dayOfYear(dt)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_dayOfYear(dt AS DateTime) AS Integer
  LET here AS Integer = __datetime_daysFromCivil(dt.date.year, dt.date.month, dt.date.day)
  LET start AS Integer = __datetime_daysFromCivil(dt.date.year, 1, 1)
  RETURN here - start + 1
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "dayOfYear",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: super::arg_hint("dayOfYear"),
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "dt",
                desc: "",
                aliases: &[],
                ty: super::ParameterType::Named("DateTime"),
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::Integer,
            errors: vec![],
            lowering: super::Lowering::Helper,
            body: super::Body::mfb(BODY, "__datetime_dayOfYear"),
        }],
    });
}
