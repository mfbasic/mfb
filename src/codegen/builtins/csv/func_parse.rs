//! `csv::parse` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:parse@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __csv_parse(value AS String, delimiter AS String, quote AS String) AS List OF List OF String
  LET delimCode AS Integer = __csv_firstCode(delimiter)
  LET quoteCode AS Integer = __csv_firstCode(quote)
  LET chars AS List OF Integer = encoding::utf32Encode(value)
  LET count AS Integer = len(chars)
  MUT rows AS List OF List OF String = []
  MUT row AS List OF String = []
  MUT fieldBuf AS List OF Integer = []
  MUT fieldStart AS Integer = 0
  MUT index AS Integer = 0
  MUT inQuotes AS Boolean = FALSE
  MUT fieldStarted AS Boolean = FALSE
  MUT wasQuoted AS Boolean = FALSE
  MUT recordPending AS Boolean = FALSE

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
      rows = collections::append(rows, row)
      row = []
      fieldBuf = []
      fieldStarted = FALSE
      wasQuoted = FALSE
      recordPending = FALSE
      index = index + __csv_separatorLength(chars, count, index)
      fieldStart = index
    ELSEIF ch = delimCode THEN
      row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
      fieldBuf = []
      fieldStarted = FALSE
      wasQuoted = FALSE
      recordPending = TRUE
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

  IF fieldStarted OR recordPending OR wasQuoted THEN
    row = collections::append(row, __csv_fieldValue(chars, fieldBuf, wasQuoted, fieldStart, index))
    rows = collections::append(rows, row)
  END IF

  RETURN rows
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_PARSE,
    return_type: ReturnType::Fixed(super::GRID_TYPE),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const PARSE: BuiltinFunction =
    BuiltinFunction::mfb("csv.parse", "parse", INTRO, DESC, &[], OV, BODY);
