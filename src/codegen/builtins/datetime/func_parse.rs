//! `datetime::parse` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/parse.md`.

const INTRO: &str = r#"Parse text into a `DateTime` using the format pattern mini-language."#;
const DESC: &str = r#"`datetime::parse` reads `value` against `pattern` and returns the `DateTime` it
describes. `pattern` uses the same token mini-language as `datetime::format`, and
`parse` is the approximate inverse of `format`: it walks `pattern` and `value`
together from left to right, consuming characters of `value` as each `pattern`
position is matched. A token (a run of one or more of the same formatting letter)
consumes and decodes the corresponding field from `value`; any other `pattern`
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
- `EEE` / `EEEE` — weekday name; the letters are consumed but not validated
- `Z` / `ZZ` / `ZZZ` — offset: the letter `Z` (or `z`) for UTC, else `+/-HH:MM` or
  `+/-HHMM` (the colon between offset hours and minutes is optional)

Numeric tokens are greedy up to their stated width but accept fewer digits, so the
minimal forms (`M`, `d`, `H`, `h`, `m`, `s`) read one or two digits and the padded
forms accept the same. Name tokens (month names, AM/PM) are matched without regard
to case. The weekday token only skips over the run of letters in `value`; it does
not check that the named weekday agrees with the parsed date.

`parse` does not range-check the decoded calendar fields the way `datetime::date`
and `datetime::time` do: an out-of-range component in `value` (for example month
13) is carried into the resulting `DateTime` rather than rejected. The one
validated numeric range is the offset token, whose magnitude must be under 24
hours.

An offset token sets the `DateTime`'s offset directly and makes the result a
fixed-offset moment, overriding `zone`. When `pattern` contains no offset token,
the `zone` argument supplies the offset: the two-argument overload defaults it to
`datetime::utc()`, and the three-argument overload resolves `value`'s civil fields
against the given `zone`. `parse` is pure: it reads no host state and has no side
effects."#;
const EX: &str = r#"Parse a date and time, interpreted as UTC:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parse("2026-06-26 09:30:00", "yyyy-MM-dd HH:mm:ss")
END SUB
```

Parse civil fields against an explicit zone:

```
IMPORT datetime

SUB main()
  LET z AS Zone = datetime::fixedOffset(-5, 0)
  LET dt AS DateTime = datetime::parse("2026-06-26 09:30", "yyyy-MM-dd HH:mm", z)
END SUB
```

An offset token in the value overrides the zone argument:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::parse("2026-06-26T09:30:00+05:30", "yyyy-MM-dd'T'HH:mm:ssZZ")
END SUB
```

Text that does not match the pattern raises `ErrInvalidFormat`:

```
IMPORT datetime

SUB main()
  LET bad AS DateTime = datetime::parse("not-a-date", "yyyy-MM-dd")
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    // Arity-dispatched: 2 args -> `__datetime_parse2`, 3 args (trailing `zone`) ->
    // `__datetime_parse3`. `select` picks by arity and yields the right rewrite.
    pkg.add_function(super::RegistryFunction {
        name: "parse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String[, Zone]"),
        implementations: vec![
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "value",
                        desc: "",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "pattern",
                        desc: "",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::Named("DateTime"),
                errors: vec![],
                lowering: super::Lowering::Helper,
                body: super::Body::Rewrite("__datetime_parse2"),
            },
            super::Implementation {
                params: vec![
                    super::Parameter {
                        name: "value",
                        desc: "",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "pattern",
                        desc: "",
                        aliases: &[],
                        ty: super::ParameterType::String,
                        default: super::DefaultValue::None,
                    },
                    super::Parameter {
                        name: "zone",
                        desc: "",
                        aliases: &[],
                        ty: super::ParameterType::Named("Zone"),
                        default: super::DefaultValue::None,
                    },
                ],
                return_type: super::ParameterType::Named("DateTime"),
                errors: vec![],
                lowering: super::Lowering::Helper,
                body: super::Body::Rewrite("__datetime_parse3"),
            },
        ],
    });
}
