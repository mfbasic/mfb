//! `datetime::parse` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

#[rustfmt::skip]
const BODY_2: &str =
r#"FUNC __datetime_parse2(value AS String, pattern AS String) AS DateTime
  RETURN __datetime_parse3(value, pattern, __datetime_utc())
END FUNC"#;

#[rustfmt::skip]
const BODY_3: &str =
r#"FUNC __datetime_parse3(value AS String, pattern AS String, zone AS Zone) AS DateTime
  RETURN __datetime_buildFromFields(__datetime_parseFields(value, pattern), zone)
END FUNC"#;

const INTRO: &str =
    r#"Parse text into a `datetime::DateTime` using the format pattern mini-language."#;
const DESC: &str = r#"`datetime::parse` reads `value` against `pattern` and returns the `datetime::DateTime` it
describes. `pattern` uses the same token mini-language as `datetime::format`, and
`parse` is the approximate inverse of `format`: it walks `pattern` and `value`
together from left to right, reading characters of `value` as each `pattern`
position is matched. A token (a run of one or more of the same formatting letter)
reads and decodes the corresponding field from `value`; any other `pattern`
character is a literal that must appear verbatim at the current position in
`value`. Single quotes escape literal text exactly as in `datetime::format` (`'T'`
matches a literal `T`, `''` matches a single apostrophe).

Fields not named by any token take defaults: year `1970`, month `1`, day `1`, and
the time `00:00:00.000000000`. The recognized tokens are:

- `yyyy` / `yy` — year; `yyyy` reads up to 4 digits, `yy` reads 2 digits and adds
  2000 (so `26` becomes `2026`)
- `M` / `MM` — month number, 1-2 digits
- `MMM` / `MMMM` — month name, short or full, case-insensitive (English)
- `d` / `dd` — day of month, 1-2 digits
- `H` / `HH` — hour on a 24-hour clock, 1-2 digits
- `h` / `hh` — hour on a 12-hour clock, 1-2 digits (combine with `a`)
- `m` / `mm` — minute, 1-2 digits
- `s` / `ss` — second, 1-2 digits
- `fff`..`fffffffff` — fractional second; reads run-length digits and scales them
  to nanoseconds (`fff` = milliseconds, `fffffffff` = nanoseconds)
- `a` — AM/PM marker, case-insensitive
- `EEE` / `EEEE` — weekday name; the letters are read but not validated
- `Z` / `ZZ` / `ZZZ` — offset: the letter `Z` (or `z`) for UTC, else `+/-HH:MM` or
  `+/-HHMM` (the colon between offset hours and minutes is optional)

Numeric tokens are greedy up to their stated width but accept fewer digits, so the
minimal forms (`M`, `d`, `H`, `h`, `m`, `s`) read one or two digits and the padded
forms accept the same. Name tokens (month names, AM/PM) are matched without regard
to case. The weekday token only skips over the run of letters in `value`; it does
not check that the named weekday agrees with the parsed date.

`parse` range-checks the decoded calendar fields against exactly the bounds
`datetime::date` and `datetime::time` enforce: `month` in `1 .. 12`, `day` in
`1 ..` the length of that month in that year (so `"2026-02-30"` and a 29
February outside a leap year are both refused), `hour` in `0 .. 23`, `minute`
and `second` in `0 .. 59`, and the fractional second in
`0 .. 999999999` nanoseconds. An out-of-range component raises
`ErrInvalidFormat` — the same code a shape mismatch raises, so one `TRAP`
catches every flavour of bad text. The bound is applied to the hour the value
actually names, after the 12-hour/AM-PM fold. The offset token's magnitude must
also be under 24 hours.

There is no rollover: `"2026-13-45"` is an error, not December-plus-one-month.
That normalization belongs to `datetime::addMonths`/`datetime::addDays`, which
are asked for it explicitly; a reader of untrusted text is not.

An offset token sets the `datetime::DateTime`'s offset directly and makes the result a
fixed-offset moment, overriding `zone`. When `pattern` contains no offset token,
the `zone` argument supplies the offset: the two-argument overload defaults it to
`datetime::utc()`, and the three-argument overload resolves `value`'s civil fields
against the given `zone`. `parse` is pure: it reads no host state and has no side
effects."#;
const EX: &str = r#"Parse a date and time, interpreted as UTC:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::parse("2026-06-26 09:30:00", "yyyy-MM-dd HH:mm:ss")
END SUB
```

Parse civil fields against an explicit zone:

```
IMPORT datetime

SUB main()
  LET z AS datetime::Zone = datetime::fixedOffset(-5, 0)
  LET dt AS datetime::DateTime = datetime::parse("2026-06-26 09:30", "yyyy-MM-dd HH:mm", z)
END SUB
```

An offset token in the value overrides the zone argument:

```
IMPORT datetime

SUB main()
  LET dt AS datetime::DateTime = datetime::parse("2026-06-26T09:30:00+05:30", "yyyy-MM-dd'T'HH:mm:ssZZ")
END SUB
```

An out-of-range calendar field raises `ErrInvalidFormat` rather than rolling
over into a different date:

```
IMPORT datetime
IMPORT io

SUB main()
  LET bad AS datetime::DateTime = datetime::parse("2026-13-45", "yyyy-MM-dd")
  io::print("accepted")
  EXIT SUB
TRAP(err)
  io::print("rejected: " & err.message)
  EXIT SUB
END TRAP
END SUB
```

prints:

```
rejected: datetime: month out of range
```

Text that does not match the pattern raises `ErrInvalidFormat`:

```
IMPORT datetime
IMPORT io

SUB main()
  LET bad AS datetime::DateTime = datetime::parse("not-a-date", "yyyy-MM-dd")
  io::print("accepted")
  EXIT SUB
TRAP(err)
  io::print("rejected: " & toString(err.code))
  EXIT SUB
END TRAP
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    // Arity-dispatched: 2 args -> `__datetime_parse2`, 3 args (trailing `zone`) ->
    // `__datetime_parse3`. `select` picks by arity and yields the right rewrite.
    pkg.add_function(super::RegistryFunction {
        name: "parse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String[, Zone]"),
        internal_only: false,
        implementations: vec![
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "value",
                        desc: "The text to parse.",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "pattern",
                        desc: "The pattern the text must match. Text that does not match raises rather than parsing as much as it can.",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::named("DateTime"),
                errors: vec![],
                body: super::Body::mfb(BODY_2, "__datetime_parse2"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "value",
                        desc: "The text to parse.",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "pattern",
                        desc: "The pattern the text must match. Text that does not match raises rather than parsing as much as it can.",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "zone",
                        desc: "The zone to interpret the parsed wall-clock reading in, when the text does not carry one itself.",
                        aliases: &[],
                        ty: super::ParameterType::named("Zone"),
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::named("DateTime"),
                errors: vec![],
                body: super::Body::mfb(BODY_3, "__datetime_parse3"),
            },
        ],
    });
}
