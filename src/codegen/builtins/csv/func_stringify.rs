//! `csv::stringify` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:stringify@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::target::shared::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __csv_stringify(value AS List OF List OF String, delimiter AS String, quote AS String, newline AS String) AS String
  MUT out AS String = ""
  MUT firstRow AS Boolean = TRUE
  FOR EACH row IN value
    IF firstRow THEN
      firstRow = FALSE
    ELSE
      out = out & newline
    END IF
    out = out & __csv_stringifyRow(row, delimiter, quote)
  NEXT
  RETURN out
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_STRINGIFY,
    return_type: ReturnType::Fixed("String"),
}];

const INTRO: &str = r#"Encode a grid of String cells as RFC-4180-aligned CSV text."#;
const DESC: &str = r#"`csv::stringify` renders a grid — a `List OF List OF String` of rows of String
cells — into a single CSV text. Rows are joined with one line feed (LF) with no
trailing newline, and the fields within a row are joined with a comma. Rows and
fields are emitted in list order, and the grid is not required to be rectangular:
each row keeps whatever field count it had.

A field is emitted quoted if and only if it contains the delimiter, the quote
character, a carriage return (CR), or a line feed (LF); otherwise it is emitted
bare. Whitespace is significant and never trimmed. Inside a quoted field every
quote character is doubled, and delimiters, CR, and LF are carried through as
ordinary data. The whole String is preserved verbatim, so a multi-byte scalar is
never split.

The optional `delimiter`, `quote`, and `newline` arguments select a dialect:
`delimiter` replaces the comma between fields, `quote` replaces the double quote
used to wrap and escape, and `newline` replaces the LF written between rows. Each
defaults to the RFC-4180 value (`,`, `"`, and LF) when omitted, so the
one-argument form is unchanged. `delimiter` and `quote` must each be a non-empty
single character.

An empty outer list stringifies to the empty String. An empty row stringifies to
an empty line, so a two-element outer list containing two empty rows produces a
lone LF. Cells are written verbatim with no type interpretation: the Strings
`"42"` and `""` are emitted as the text `42` and an empty field.

For any grid `x`, `csv::parse(csv::stringify(x))` yields a grid whose cells equal
those of `x`, with one normalization: a trailing empty row produced only by
separator placement is not reintroduced, and a CRLF separator in the original
text is normalized to LF on output.

The sole argument is named `value`, so it can be supplied positionally or as the
keyword argument `value :=`. `csv::stringify` does not mutate `value` and has no
side effects."#;
const EX: &str = r#"Serialize a grid, quoting only the cell that needs it:

```
IMPORT csv

SUB main()
  LET text AS String = csv::stringify([["name", "age"], ["Grace", "Hop,per"]])
END SUB
```

Pass the argument by name:

```
IMPORT csv

SUB main()
  LET out AS String = csv::stringify(value :=)
END SUB
```"#;

pub(crate) const STRINGIFY: BuiltinFunction =
    BuiltinFunction::mfb("csv.stringify", "stringify", INTRO, DESC, &[], OV, BODY).with_example(EX);
