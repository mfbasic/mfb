//! `datetime::time` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/time.md`.

const INTRO: &str = r#"Validate and build a time-of-day `Time` from hour, minute, second, and sub-second components."#;
const DESC: &str = r#"`datetime::time` builds a `Time` of day from its `hour`, `minute`, `second`, and
sub-second (`nanos`) components. A `Time` names a position within a single
24-hour day and carries no calendar date and no zone; pair it with a `Date`
through `datetime::civil` to build a zoned `DateTime`.

The constructor validates each component against its civil range before
returning, and there is no normalization or wrap-around: an out-of-range
component is an error, not silently carried into the next unit. `hour` must be
in `0 .. 23`, where `0` is midnight and `23` is the final hour of the day.
`minute` and `second` must each be in `0 .. 59`; the model has no leap seconds,
so `60` is never a valid second. `nanos` is the sub-second remainder and must be
in `0 .. 999_999_999`.

`second` and `nanos` default to `0`, so a two-argument call names the top of a
minute and a three-argument call names the top of a second. Unlike
`datetime::instant` and `datetime::duration`, `time` is not overloaded but a
single signature with trailing defaults, so the defaults apply and you may omit
`second`, or both `second` and `nanos`.

`time` is pure: the same arguments always yield the same `Time`, and it has no
side effects."#;
const EX: &str = r#"Construct a time at the top of a minute (`second` and `nanos` default to `0`):

```
IMPORT datetime

SUB main()
  LET t AS Time = datetime::time(9, 30)
END SUB
```

Construct a time with whole seconds:

```
IMPORT datetime

SUB main()
  LET t AS Time = datetime::time(23, 59, 59)
END SUB
```

Combine a date and time into a zoned `DateTime`:

```
IMPORT datetime

SUB main()
  LET d AS Date = datetime::date(2026, 6, 26)
  LET t AS Time = datetime::time(9, 30)
  LET dt AS DateTime = datetime::civil(d, t, datetime::utc())
END SUB
```

An out-of-range field raises `ErrInvalidArgument`:

```
IMPORT datetime

SUB main()
  LET bad AS Time = datetime::time(24, 0)
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_time(hour AS Integer, minute AS Integer, second AS Integer, nanos AS Integer) AS Time
  IF hour < 0 OR hour > 23 THEN
    FAIL error(77050002, "datetime: hour out of range")
  END IF
  IF minute < 0 OR minute > 59 THEN
    FAIL error(77050002, "datetime: minute out of range")
  END IF
  IF second < 0 OR second > 59 THEN
    FAIL error(77050002, "datetime: second out of range")
  END IF
  IF nanos < 0 OR nanos > 999999999 THEN
    FAIL error(77050002, "datetime: nanos out of range")
  END IF
  RETURN Time[hour, minute, second, nanos]
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    // `second`/`nanos` are optional (default 0). They widen arity here; the actual
    // `0` padding is injected by the retained `default_argument_padding` so the
    // 4-arg `__datetime_time` body always receives every component.
    super::single(
        pkg,
        "time",
        INTRO,
        DESC,
        EX,
        vec![
            super::req("hour", super::int()),
            super::req("minute", super::int()),
            super::optional("second", super::int()),
            super::optional("nanos", super::int()),
        ],
        super::named("Time"),
        BODY,
        "__datetime_time",
    );
}
