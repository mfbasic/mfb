//! `csv::parseStream` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (mirrors collections/encoding func_*.rs). Source-backed
//! (`Implementation::Mfb`): the `__csv_*` body lives here and replaces a
//! `'@@MFB_BODY:parseStream@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __csv_parseStream(value AS String, delimiter AS String, quote AS String) AS CsvReader
  LET chars AS List OF Integer = encoding::utf32Encode(value)
  RETURN CsvReader[chars, len(chars), 0, __csv_firstCode(delimiter), __csv_firstCode(quote)]
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_PARSE_STREAM,
    return_type: ReturnType::Fixed(super::READER_TYPE),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const PARSE_STREAM: BuiltinFunction =
    BuiltinFunction::mfb("csv.parseStream", "parseStream", INTRO, DESC, &[], OV, BODY);
