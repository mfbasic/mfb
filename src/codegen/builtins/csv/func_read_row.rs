//! `csv::readRow` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:readRow@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
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

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_NEXT,
    return_type: ReturnType::Fixed(super::ROW_TYPE),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const READ_ROW: BuiltinFunction =
    BuiltinFunction::mfb("csv.readRow", "readRow", INTRO, DESC, &[], OV, BODY);
