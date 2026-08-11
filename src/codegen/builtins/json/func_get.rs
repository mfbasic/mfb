//! `json::get` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:get@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_get(value AS Json, path AS List OF String) AS Json
  MUT current AS Json = value
  FOR EACH key IN path
    MUT nextValue AS Json = current
    LET currentValue AS Json = current
    MATCH currentValue
      CASE JsonObj(obj)
        nextValue = collections::get(obj.fields, key)
      CASE ELSE
        FAIL error(77050004, "Requested item, key, file, or resource was not found.")
    END MATCH
    current = nextValue
  NEXT
  RETURN current
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_GET,
    return_type: ReturnType::Fixed("Json"),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const GET: BuiltinFunction =
    BuiltinFunction::mfb("json.get", "get", INTRO, DESC, &[], OV, BODY);
