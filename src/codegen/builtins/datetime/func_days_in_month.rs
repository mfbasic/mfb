//! `datetime::daysInMonth` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/daysInMonth.md`.

const INTRO: &str = r#"The number of days in a calendar month."#;
const DESC: &str = r#"`datetime::daysInMonth` returns the number of days in the given `month` of the
given `year` under the proleptic-Gregorian calendar. The result is `31` for
January, March, May, July, August, October, and December; `30` for April, June,
September, and November; and `28` or `29` for February depending on whether
`year` is a leap year.

February's length is decided by applying the leap-year rule to `year`: a leap
February has `29` days, otherwise it has `28`. The leap rule is purely
arithmetic on the year number (divisible by `4`, except century years that are
not divisible by `400`), so it extends indefinitely into the past and future and
treats zero and negative year numbers by the same divisibility test.


Only February consults `year`; for every other month the result depends solely
on `month`, and `year` is ignored. The `month` argument is not range-checked:
any value that is not `2`, `4`, `6`, `9`, or `11` yields `31`, so out-of-range
month numbers do not raise an error but return `31` by falling through to the
default case.

The function reads no time zone, `Instant`, or current clock value and has no
side effects."#;
const EX: &str = r#"Length of common and leap-year February:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::daysInMonth(2023, 2)))   ' 28
  io::print(toString(datetime::daysInMonth(2024, 2)))   ' 29 (leap year)
END SUB
```

Lengths of the other months ignore the year:

```
IMPORT datetime
IMPORT io

SUB main()
  io::print(toString(datetime::daysInMonth(2026, 1)))   ' 31
  io::print(toString(datetime::daysInMonth(2026, 4)))   ' 30
END SUB
```

Clamp a day-of-month to the end of its month:

```
IMPORT datetime
IMPORT io

SUB main()
  LET year AS Integer = 2024
  LET month AS Integer = 2
  MUT day AS Integer = 31
  LET last AS Integer = datetime::daysInMonth(year, month)
  IF day > last THEN
    day = last
  END IF
  io::print(toString(day))   ' 29
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_daysInMonth(year AS Integer, month AS Integer) AS Integer
  IF month = 2 THEN
    IF __datetime_isLeapYear(year) THEN
      RETURN 29
    END IF
    RETURN 28
  END IF
  IF month = 4 OR month = 6 OR month = 9 OR month = 11 THEN
    RETURN 30
  END IF
  RETURN 31
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "daysInMonth",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer, Integer"),
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "year",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                },
                super::Parameter {
                    name: "month",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Integer,
                    default: super::DefaultValue::None,
                },
            ],
            return_type: super::ParameterType::Integer,
            errors: vec![],
            lowering: super::Lowering::Helper,
            body: super::Body::mfb(BODY, "__datetime_daysInMonth"),
        }],
    });
}
