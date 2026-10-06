//! `csv::readRow` — descriptor entry + MFBASIC source body.
//!
//! Reads ONE record from an open `File` and leaves the file just past it
//! (bug-809). The reader state is the `File` itself — its position, advanced in
//! place by the host — so a loop over a document never copies the document, and
//! costs time linear in its size. (The previous `csv::CsvReader` value carried the
//! whole decoded input and was rebuilt on every call: quadratic.) The record is
//! tokenized by the shared `__csv_scan`, stopped after one record.
//! Body byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Read the next CSV record from an open `File`."#;

const DESC: &str = r#"`csv::readRow` reads exactly one record from `file`, starting at the file's
current position, and returns its cells as a `List OF String`. The file's
position moves to just past the record, so the next `csv::readRow` reads the
record after it. Only that one record is held in memory, which is what makes
`csv::readRow` the way to work through a document too large to hold as a whole
`List OF List OF String`: the time a loop takes grows in step with the size of
the file.

A record normally ends at the end of its line. A quoted field may contain line
breaks, so a record whose quoted field is still open at the end of a line goes on
to the next line, and as many more as it takes for the quote to close; the line
breaks inside the field are kept exactly as written (LF or CRLF).

The records `csv::readRow` returns, in order, are exactly the rows `csv::parse`
gives for the same text and dialect, including the RFC-4180 rules for quoting,
doubled quotes, LF and CRLF record separators, and a final record with no line
break after it. The optional `delimiter` and `quote` select the dialect exactly as
for `csv::parse` (defaults `,` and `"`); each must be non-empty — an empty one
raises `ErrInvalidFormat` without reading anything — and only its first Unicode
scalar is used. Pass the same dialect on every call for one document.

At end of input — when no record is left to read — `csv::readRow` raises
`ErrEndOfFile`, as `fs::readLine` does; test `fs::eof(file)` first to stop a loop
cleanly. A record that is not well-formed CSV raises `ErrInvalidFormat`: text
after a field's closing quote other than a delimiter or a record separator, or a
quoted field that is still open when the input ends. The file must hold valid
UTF-8; a line that is not raises `ErrEncoding`. Reading can also fail the way
`fs::readLine` does: `ErrReadFailed` when the read itself fails,
`ErrResourceClosed` for a closed file, and `ErrResourceMoved` for one handed to
another thread. `ErrOutOfMemory` means the program ran out of memory.

`file` can be any `File` opened in a mode that permits reading — one from
`fs::openFile`, or `fs::openData` to read CSV text already held in a `String`.
`csv::readRow` reads it with the same per-file read buffer `fs::readLine` uses, so
the two can be mixed on one file: skip a preamble with `fs::readLine`, then read
records."#;

const EX: &str = r#"Process a CSV file one record at a time:

```
IMPORT csv
IMPORT collections
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  fs::writeText("people.csv", "name,age\nAda,36\nGrace,45\n")
  RES f = fs::openFile("people.csv")
  LET header AS List OF String = csv::readRow(f)
  WHILE NOT fs::eof(f)
    LET row AS List OF String = csv::readRow(f)
    io::print(collections::get(row, 0) & " is " & collections::get(row, 1))
  END WHILE
END FUNC
```

Read records from text already in memory, with a semicolon delimiter:

```
IMPORT csv
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  RES f = fs::openData("a;b\n\"multi\nline\";c\n", "r")
  MUT count AS Integer = 0
  WHILE NOT fs::eof(f)
    LET row AS List OF String = csv::readRow(f, ";")
    count = count + 1
  END WHILE
  io::print("records=" & toString(count))
END FUNC
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __csv_readRow(file AS fs::File, delimiter AS String, quote AS String) AS List OF String
  LET delimCode AS Integer = __csv_firstCode(delimiter)
  LET quoteCode AS Integer = __csv_firstCode(quote)
  LET line AS String = fs::readLineRaw(file)
  IF line = "" THEN
    FAIL error(77020003, "csv: no record left before the end of the file")
  END IF
  RETURN collections::get(__csv_scan(file, line, delimCode, quoteCode, 1), 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readRow",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File[, String, String]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "file",
                    desc: "An open `File` positioned at the start of a record, opened in a mode \
                           that permits reading. Its position moves past the record read.",
                    aliases: &[],
                    ty: ParameterType::named(crate::codegen::builtins::fs::FILE_TYPE_ID),
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
            ],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![
                "ErrEndOfFile",
                "ErrInvalidFormat",
                "ErrEncoding",
                "ErrOutOfMemory",
                "ErrReadFailed",
                "ErrResourceClosed",
                "ErrResourceMoved",
            ],
            body: Body::mfb(FUNC_BODY, "__csv_readRow"),
        }],
    });
}
