//! `csv::stringify` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:stringify@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

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
one-argument form is unchanged. `delimiter` and `quote` must be non-empty (an empty one raises `ErrInvalidFormat`). Only the first Unicode scalar is meaningful to `csv::parse` and `csv::readRow`, which `stringify` does not enforce: it writes the whole text, so pass exactly one scalar if the output must be read back.

An empty outer list stringifies to the empty String. An empty row stringifies to
an empty line, so a two-element outer list containing two empty rows produces a
lone LF. Cells are written verbatim with no type interpretation: the Strings
`"42"` and `""` are emitted as the text `42` and an empty field.

For any grid `x`, with the default newline (or CRLF), `csv::parse(csv::stringify(x))` yields a grid whose cells equal those of `x`, except that a trailing empty row produced only by separator placement is not reintroduced. Going the other way, `csv::stringify(csv::parse(text))` reproduces `text` except that CRLF record separators come back as LF and a single trailing separator is dropped.

The grid is the parameter `value` and may be passed positionally or as `value :=`; `delimiter`, `quote` and `newline` may likewise be passed by name. `csv::stringify` does not mutate `value` and has no
side effects. `ErrOutOfMemory` means the program ran out of memory."#;

const EX: &str = r#"Serialize a grid, quoting only the cell that needs it:

```
IMPORT csv

FUNC main() AS Nothing
  LET text AS String = csv::stringify([["name", "age"], ["Grace", "Hop,per"]])
END FUNC
```

Pass the argument by name:

```
IMPORT csv

FUNC main() AS Nothing
  LET out AS String = csv::stringify(value := [["name", "age"], ["Grace", "Hop,per"]])
END FUNC
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __csv_stringify(value AS List OF List OF String, delimiter AS String, quote AS String, newline AS String) AS String
  LET checkDelimiter AS Integer = __csv_firstCode(delimiter)
  LET checkQuote AS Integer = __csv_firstCode(quote)
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stringify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The grid of rows of String cells to serialize.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::list_of(ParameterType::String)),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "delimiter",
                    desc: "The single character that separates fields. Defaults to `,`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::String,
                        expr: super::DEFAULT_DELIMITER,
                    },
                },
                Parameter {
                    name: "quote",
                    desc: "The single character that wraps a field and, doubled, escapes itself. Defaults to `\"`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::String,
                        expr: super::DEFAULT_QUOTE,
                    },
                },
                Parameter {
                    name: "newline",
                    desc: "The text written between rows. Defaults to a line feed.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::String,
                        expr: super::DEFAULT_NEWLINE,
                    },
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrInvalidFormat", "ErrOutOfMemory"],
            body: Body::mfb(FUNC_BODY, "__csv_stringify"),
        }],
    });
}
