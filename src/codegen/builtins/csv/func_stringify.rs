//! `csv::stringify` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:stringify@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

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

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const STRINGIFY: BuiltinFunction =
    BuiltinFunction::mfb("csv.stringify", "stringify", INTRO, DESC, &[], OV, BODY);
