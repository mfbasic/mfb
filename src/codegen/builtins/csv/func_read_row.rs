//! `csv::readRow` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:readRow@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Read the next record from a streaming CSV reader."#;

const DESC: &str = r#"`csv::readRow` parses exactly one record starting at `reader`'s cursor and returns
a `csv::CsvRow` with three fields: `fields` (the record's cells, a `List OF String`),
`reader` (a new `csv::CsvReader` advanced past the record, to pass to the next
`csv::readRow`), and `done` (`TRUE` when the reader was already at end of input,
in which case `fields` is empty). Reading is purely functional — the input
`reader` is not modified; each call returns the advanced reader to thread into the
next call.

The records `readRow` yields, in order, are identical to those `csv::parse`
produces for the same input and dialect, including the RFC-4180 rules for quoting,
doubled quotes, CR/LF and CRLF record separators, and the suppression of a
trailing empty row. The dialect is fixed when the reader is opened by
`csv::parseStream`.

A record that is not well-formed CSV — a quoted field with no closing quote, or
text after a field's closing quote other than a delimiter or a record separator —
raises `ErrInvalidFormat`, as `csv::parse` does; so does a scalar in `chars` that
is not a Unicode scalar value.

A `csv::CsvReader` built directly, rather than by `csv::parseStream` or a previous
`csv::readRow`, raises `ErrInvalidArgument` when its `count` is not the length of
`chars`, or its `index` is negative or past `count`. Building the returned row
needs memory; `ErrOutOfMemory` means the program ran out of memory.

`csv::readRow` has no side effects."#;

const EX: &str = r#"Count the rows of a large CSV without materializing the grid:

```
IMPORT csv
IMPORT io

FUNC main() AS Nothing
  MUT count AS Integer = 0
  MUT row AS csv::CsvRow = csv::readRow(csv::parseStream("1,a\n2,b\n3,c"))
  WHILE row.done = FALSE
    count = count + 1
    row = csv::readRow(row.reader)
  END WHILE
  io::print("rows=" & toString(count))
END FUNC
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __csv_next(reader AS CsvReader) AS CsvRow
  LET chars AS List OF Integer = reader.chars
  LET count AS Integer = reader.count
  LET delimCode AS Integer = reader.delimCode
  LET quoteCode AS Integer = reader.quoteCode
  MUT index AS Integer = reader.index
  IF count <> len(chars) THEN
    FAIL error(77050002, "csv: the reader's count field is not the length of its chars")
  END IF
  IF index < 0 OR index > count THEN
    FAIL error(77050002, "csv: the reader's index field is outside 0 .. count")
  END IF
  IF index >= count THEN
    RETURN CsvRow[[], reader, TRUE]
  END IF
  MUT row AS List OF String = []
  MUT fieldBuf AS List OF Integer = []
  MUT fieldStart AS Integer = index
  MUT inQuotes AS Boolean = FALSE
  MUT fieldStarted AS Boolean = FALSE
  MUT wasQuoted AS Boolean = FALSE
  WHILE index < count
    LET ch AS Integer = collections::get(chars, index)
    IF inQuotes THEN
      IF ch = quoteCode THEN
        IF __csv_isDoubledQuote(chars, count, index, quoteCode) THEN
          fieldBuf = collections::append(fieldBuf, quoteCode)
          index = index + 2
        ELSE
          inQuotes = FALSE
          wasQuoted = TRUE
          index = index + 1
        END IF
      ELSE
        fieldBuf = collections::append(fieldBuf, ch)
        index = index + 1
      END IF
    ELSEIF __csv_separatorLength(chars, count, index) > 0 THEN
      row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
      index = index + __csv_separatorLength(chars, count, index)
      RETURN CsvRow[row, CsvReader[chars, count, index, delimCode, quoteCode], FALSE]
    ELSEIF ch = delimCode THEN
      row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
      fieldBuf = []
      fieldStarted = FALSE
      wasQuoted = FALSE
      index = index + 1
      fieldStart = index
    ELSEIF wasQuoted THEN
      FAIL error(77050003, "invalid CSV format")
    ELSEIF ch = quoteCode AND fieldStarted = FALSE THEN
      inQuotes = TRUE
      fieldStarted = TRUE
      index = index + 1
    ELSE
      fieldStarted = TRUE
      index = index + 1
    END IF
  END WHILE
  IF inQuotes THEN
    FAIL error(77050003, "invalid CSV format")
  END IF
  row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
  RETURN CsvRow[row, CsvReader[chars, count, index, delimCode, quoteCode], FALSE]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readRow",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "reader",
                desc: "A reader from `csv::parseStream` or a previous `csv::readRow`. A reader built directly whose `count` is not the length of `chars`, or whose `index` is negative or past `count`, raises `ErrInvalidArgument`.",
                aliases: &[],
                ty: ParameterType::named("CsvReader"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("CsvRow"),
            errors: vec!["ErrInvalidArgument", "ErrInvalidFormat", "ErrOutOfMemory"],
            body: Body::mfb(FUNC_BODY, "__csv_next"),
        }],
    });
}
