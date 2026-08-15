//! `datetime::format` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/format.md`.

const INTRO: &str = r#"Render a `DateTime` as text with the pattern mini-language."#;
const DESC: &str = r#"`datetime::format` renders the fields of `dt` as text by walking `pattern` from
left to right and emitting, for each position, either a literal character or the
value selected by a formatting token. The result is a freshly built `String`;
`dt` is read only and is not modified. An empty pattern yields the empty string.


A token is a run of one or more of the same ASCII letter (`A`–`Z` or `a`–`z`);
the run length selects the width or style of the field. Any character that is
not an ASCII letter is copied to the output verbatim, so separators such as
spaces, dashes, colons, and slashes appear literally. A run of a letter that is
not one of the recognized tokens below is an error, not literal text: to emit a
letter literally, wrap it in single quotes (`'T'` produces a literal `T`); to
emit a literal apostrophe, write two single quotes (`''`).


The recognized tokens are:

- `yy` — last two digits of the year, zero-padded; any other run of `y`
  zero-pads the full year to the run length (`yyyy` pads to at least 4 digits)
- `M` / `MM` — month number, minimal (1-12) / 2-digit
- `MMM` — month name, short (English); any run of 4 or more `M` gives the full name
- `d` — day of month, minimal; any run of 2 or more `d` gives the 2-digit form
- `H` — hour on a 24-hour clock (0-23), minimal; 2 or more `H` gives 2-digit
- `h` — hour on a 12-hour clock (1-12), minimal; 2 or more `h` gives 2-digit
- `m` — minute, minimal; 2 or more `m` gives 2-digit
- `s` — second, minimal; 2 or more `s` gives 2-digit
- `f` .. `fffffffff` — fractional second, fixed to the run length (`fff` = ms,
  `ffffff` = us, `fffffffff` = ns)
- `a` — AM/PM marker (`AM` before noon, `PM` at or after noon)
- `E` .. `EEE` — weekday name, short (English); any run of 4 or more `E` gives
  the full name
- `Z` — offset: the letter `Z` when the offset is zero, else `+/-HH:MM`
- `ZZ` — offset, always `+/-HH:MM` (`Z` is never substituted)
- `ZZZ` and longer — offset, `+/-HHMM` with no colon

The fractional-second token renders the `nanos` of `dt.time` as 9 digits and
keeps the leading run-length digits, so `fff` yields milliseconds, `ffffff`
microseconds, and `fffffffff` nanoseconds. Month, weekday, and AM/PM names are
English. The offset tokens read `dt.offset`, the resolved UTC offset carried by
`dt`.

Inside single quotes every character, including formatting letters, is copied
literally until the closing quote; an opening quote with no matching close runs
to the end of `pattern`. `datetime::format` is pure: it reads no host state and
has no side effects."#;
const EX: &str = r#"Render a `DateTime` with a full date, time, and offset:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET text AS String = datetime::format(dt, "EEEE yyyy-MM-dd HH:mm:ss Z")
END SUB
```

Use single quotes to include literal letters in the output:

```
IMPORT datetime

SUB main()
  LET dt AS DateTime = datetime::toUtc(datetime::now())
  LET text AS String = datetime::format(dt, "yyyy-MM-dd'T'HH:mm:ss")
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_format(dt AS DateTime, pattern AS String) AS String
  LET n AS Integer = len(pattern)
  MUT out AS String = ""
  MUT i AS Integer = 0
  WHILE i < n
    LET ch AS String = strings::mid(pattern, i, 1)
    IF ch = "'" THEN
      IF i + 1 < n AND strings::mid(pattern, i + 1, 1) = "'" THEN
        out = out & "'"
        i = i + 2
      ELSE
        MUT j AS Integer = i + 1
        WHILE j < n AND strings::mid(pattern, j, 1) <> "'"
          out = out & strings::mid(pattern, j, 1)
          j = j + 1
        END WHILE
        i = j + 1
      END IF
    ELSEIF __datetime_isLetter(ch) THEN
      MUT runLen AS Integer = 1
      WHILE i + runLen < n AND strings::mid(pattern, i + runLen, 1) = ch
        runLen = runLen + 1
      END WHILE
      out = out & __datetime_formatToken(dt, ch, runLen)
      i = i + runLen
    ELSE
      out = out & ch
      i = i + 1
    END IF
  END WHILE
  RETURN out
END FUNC"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "format",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("DateTime, String"),
        implementations: vec![super::Implementation {
            params: vec![
                super::Parameter {
                    name: "dt",
                    desc: "",
                    aliases: &[],
                    ty: super::ParameterType::Named("DateTime"),
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
            return_type: super::ParameterType::String,
            errors: vec![],
            lowering: super::Lowering::Helper,
            body: super::Body::mfb(BODY, "__datetime_format"),
        }],
    });
}
