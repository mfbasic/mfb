//! `csv::parse` — descriptor entry + MFBASIC source body.
//!
//! Two overloads over ONE tokenizer, `__csv_scan` (bug-809): the `String` form
//! hands it the whole text with an already-ended `File`; the `File` form hands it
//! the file and no text, so every record arrives a line at a time.
//! Body byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Parse CSV text, or a CSV file, into a grid of String cells."#;

const DESC: &str = r#"`csv::parse` reads a whole CSV document and returns it as a
`List OF List OF String`: an ordered list of rows, each an ordered list of String
cells. `value` is either the CSV text itself, as a `String`, or an open `File` to
read it from.

Given a `File`, `csv::parse` reads from the file's current position to its end,
so the file is at end of input afterwards (`fs::eof` reports `TRUE`). The file
stays open; close it as usual. Records that were already read with
`csv::readRow` are not read again, so a header row can be taken with
`csv::readRow` and the rest parsed in one call. Given a `String`, the result is
exactly what parsing a `File` holding that text would give. To handle a large
document without building the whole grid, read it one record at a time with
`csv::readRow` instead.

Parsing is done in terms of Unicode scalars, never bytes, so a multi-byte
character is never split. Every character CSV treats as structural — comma,
quote, CR, LF — is ASCII, so the result is the same either way. A `File` must hold
valid UTF-8; a line that is not raises `ErrEncoding`.

The dialect is RFC-4180-aligned. The field delimiter defaults to a comma (scalar
`44`) but can be overridden with the optional `delimiter` argument; the quote
character defaults to the double quote (`34`) but can be overridden with the
optional `quote` argument. Each must be non-empty — an empty one raises
`ErrInvalidFormat` before anything is read — and parsing uses only its first
Unicode scalar; `csv::stringify` writes the whole text, so pass exactly one scalar
when the same dialect must read back what it wrote. A record separator is a line
feed (LF, `10`) or a carriage-return/line-feed pair (CRLF, `13` then `10`)
regardless of dialect; a bare CR not followed by LF is ordinary data inside the
current field. A field may be wrapped in the quote character: the opening quote
must be the first character of the field, inside a quoted field a literal quote is
written by doubling it, and delimiters, CR, and LF are ordinary data, so a quoted
field may span lines and keeps its line breaks exactly as written. The closing
quote must be immediately followed by the delimiter, a record separator, or the
end of input. Whitespace is significant and never trimmed.

Cells are plain Strings with no type inference and no null: `42`, `true`, and an
empty field parse to the Strings `"42"`, `"true"`, and `""`. Callers that want
numbers convert explicitly with `toFloat` or `toInteger`. Rows are not required
to be rectangular; each row keeps whatever field count it had. A single trailing
record separator does not create an empty final row, so `"a\nb\n"` parses to two
rows, while two consecutive separators do produce an empty row in the middle.
Empty input parses to zero rows. There is no header concept — every parsed line
is an ordinary row, and cells are read positionally with `collections::get`.

A quoted field with no closing quote, or text after a field's closing quote other
than a delimiter or a record separator, raises `ErrInvalidFormat`. Reading a
`File` can also fail the way `fs::readLine` does: `ErrReadFailed` when the read
itself fails, `ErrResourceClosed` for a closed file, and `ErrResourceMoved` for
one handed to another thread. For a `String`, `ErrWriteFailed` or `ErrReadFailed`
mean the system could not hold the text in memory to read it back.
`ErrOutOfMemory` means the program ran out of memory.

A `String` argument may also be supplied by the name `text`. `csv::parse` does
not change `value`; given a `File`, it moves the file's position to the end."#;

const EX: &str = r#"Parse a two-column document with a quoted cell:

```
IMPORT csv

FUNC main() AS Nothing
  LET doc AS List OF List OF String = csv::parse("name,age\n\"Ada, Countess\",36")
END FUNC
```

Pass the argument by name:

```
IMPORT csv

FUNC main() AS Nothing
  LET rows AS List OF List OF String = csv::parse(text := "a,b,c")
END FUNC
```

Take the header row, then parse the rest of a file:

```
IMPORT csv
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  fs::writeText("people.csv", "name,age\nAda,36\nGrace,45\n")
  RES f = fs::openFile("people.csv")
  LET header AS List OF String = csv::readRow(f)
  LET rows AS List OF List OF String = csv::parse(f)
  io::print(toString(len(header)) & " columns, " & toString(len(rows)) & " rows")
END FUNC
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __csv_parse(value AS String, delimiter AS String, quote AS String) AS List OF List OF String
  LET delimCode AS Integer = __csv_firstCode(delimiter)
  LET quoteCode AS Integer = __csv_firstCode(quote)
  RES ended AS fs::File = fs::openData("", "r")
  RETURN __csv_scan(ended, value, delimCode, quoteCode, 0)
END FUNC"#;

#[rustfmt::skip]
const FUNC_BODY_FILE: &str =
r#"FUNC __csv_parseFile(RES value AS fs::File, delimiter AS String, quote AS String) AS List OF List OF String
  LET delimCode AS Integer = __csv_firstCode(delimiter)
  LET quoteCode AS Integer = __csv_firstCode(quote)
  RETURN __csv_scan(value, "", delimCode, quoteCode, 0)
END FUNC"#;

fn dialect_params() -> [Parameter; 2] {
    [
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
    ]
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let [delimiter, quote] = dialect_params();
    let [delimiter_file, quote_file] = dialect_params();
    pkg.add_function(RegistryFunction {
        name: "parse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String or File[, String, String]"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "The UTF-8 CSV text to parse.",
                        aliases: &["text"],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    delimiter,
                    quote,
                ],
                return_type: ParameterType::list_of(ParameterType::list_of(ParameterType::String)),
                errors: vec![
                    "ErrInvalidFormat",
                    "ErrOutOfMemory",
                    "ErrReadFailed",
                    "ErrWriteFailed",
                ],
                body: Body::mfb(FUNC_BODY, "__csv_parse"),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "An open `File` holding UTF-8 CSV text, opened in a mode that \
                               permits reading. Parsing starts at its current position and reads \
                               to the end.",
                        aliases: &[],
                        ty: ParameterType::named(crate::codegen::builtins::fs::FILE_TYPE_ID),
                        default: DefaultValue::None,
                    },
                    delimiter_file,
                    quote_file,
                ],
                return_type: ParameterType::list_of(ParameterType::list_of(ParameterType::String)),
                errors: vec![
                    "ErrInvalidFormat",
                    "ErrEncoding",
                    "ErrOutOfMemory",
                    "ErrReadFailed",
                    "ErrResourceClosed",
                    "ErrResourceMoved",
                ],
                body: Body::mfb(FUNC_BODY_FILE, "__csv_parseFile"),
            },
        ],
    });
}
