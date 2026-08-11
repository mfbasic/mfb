//! `json::parse` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:parse@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parse(value AS String) AS Json
  LET chars AS List OF String = strings::graphemes(value)
  ' bug-422: depth 0 seeds the structural nesting-depth guard threaded through
  ' the value/array/object parsers below.
  LET parsed AS __json_Node = __json_parseValue(chars, __json_skipWhitespace(chars, 0), 0)
  LET endIndex AS Integer = __json_skipWhitespace(chars, parsed.index)
  IF endIndex <> len(chars) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  RETURN parsed.value
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_PARSE,
    return_type: ReturnType::Fixed("Json"),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const PARSE: BuiltinFunction =
    BuiltinFunction::mfb("json.parse", "parse", INTRO, DESC, &[], OV, BODY);
