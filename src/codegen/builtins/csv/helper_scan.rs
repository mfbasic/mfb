//! `__csv_scan` — the ONE CSV tokenizer (bug-809), shared by `csv::parse(String)`,
//! `csv::parse(fs::File)` and `csv::readRow` through two input adapters:
//!
//! * **String** — the whole document arrives as `text`, decoded to scalars once,
//!   and `file` is an empty `fs::openData` file, so the first refill is the end of
//!   input. This is the shape the pre-bug-809 `__csv_parse` loop had, and costs
//!   what it cost.
//! * **File** — `text` is empty (`parse`) or the record's first raw line
//!   (`readRow`), and every refill is the next line from the internal
//!   `fs::readLineRaw`, which keeps each line's exact LF/CRLF terminator and
//!   returns `""` only at the end of input. A quoted field still open at the end
//!   of a line simply continues into the next refill, with its CR/LF bytes intact.
//!
//! Records end at a separator; `maxRecords` (0 = no limit) stops the scan right
//! after the record that reaches it, before any further read — `readRow` passes 1
//! and leaves the file just past its record. Routing the `String` form through a
//! full `fs::openData` copy instead (one adapter) was measured 6–9% slower than
//! the old `__csv_parse` on 50k/200k-row documents (macos-aarch64, best of 15),
//! so the String adapter keeps the one-pass decode.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Scan `text`, then each further raw line of `file`, into records; "" from the
' file is the end of input. Stops after `maxRecords` records (0 = all).
FUNC __csv_scan(RES file AS fs::File, text AS String, delimCode AS Integer, quoteCode AS Integer, maxRecords AS Integer) AS List OF List OF String
  MUT chars AS List OF Integer = encoding::utf32Encode(text)
  MUT count AS Integer = len(chars)
  MUT rows AS List OF List OF String = []
  MUT row AS List OF String = []
  MUT fieldBuf AS List OF Integer = []
  MUT fieldStart AS Integer = 0
  MUT index AS Integer = 0
  MUT taken AS Integer = 0
  MUT inQuotes AS Boolean = FALSE
  MUT fieldStarted AS Boolean = FALSE
  MUT wasQuoted AS Boolean = FALSE
  MUT recordPending AS Boolean = FALSE
  MUT done AS Boolean = FALSE

  WHILE done = FALSE
    WHILE index < count AND done = FALSE
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
        taken = taken + 1
        IF taken = maxRecords THEN
          done = TRUE
        END IF
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
    IF done = FALSE THEN
      LET more AS String = fs::readLineRaw(file)
      IF more = "" THEN
        done = TRUE
      ELSE
        chars = encoding::utf32Encode(more)
        count = len(chars)
        index = 0
        fieldStart = 0
      END IF
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("csv_scan", BODY));
}
