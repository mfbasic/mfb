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
a `CsvRow` with three fields: `fields` (the record's cells, a `List OF String`),
`reader` (a new `CsvReader` advanced past the record, to pass to the next
`csv::readRow`), and `done` (`TRUE` when the reader was already at end of input,
in which case `fields` is empty). Reading is purely functional — the input
`reader` is not modified; each call returns the advanced reader to thread into the
next call.

The records `readRow` yields, in order, are identical to those `csv::parse`
produces for the same input and dialect, including the RFC-4180 rules for quoting,
doubled quotes, CR/LF and CRLF record separators, and the suppression of a
trailing empty row. The dialect is fixed when the reader is opened by
`csv::parseStream`.

`csv::readRow` has no side effects."#;

const EX: &str = r#"Count the rows of a large CSV without materializing the grid:

```
IMPORT csv
IMPORT io

SUB main()
  MUT count AS Integer = 0
  MUT row AS CsvRow = csv::readRow(csv::parseStream("1,a\n2,b\n3,c"))
  WHILE row.done = FALSE
    count = count + 1
    row = csv::readRow(row.reader)
  END WHILE
  io::print("rows=" & toString(count))
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __csv_next(reader AS CsvReader) AS CsvRow
  LET chars AS List OF Integer = reader.chars
  LET count AS Integer = reader.count
  LET delimCode AS Integer = reader.delimCode
  LET quoteCode AS Integer = reader.quoteCode
  MUT index AS Integer = reader.index
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

pub(super) fn register(pkg: &mut RegistryPackage) {
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
                desc: "A reader from `csv::parseStream` or a previous `csv::readRow`.",
                aliases: &[],
                ty: ParameterType::Named("CsvReader"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Named("CsvRow"),
            errors: vec!["ErrInvalidFormat"],
            body: Body::mfb(FUNC_BODY, "__csv_next"),
        }],
    });
}
