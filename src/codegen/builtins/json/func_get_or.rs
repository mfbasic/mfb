//! `json::getOr` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:getOr@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_getOr(value AS Json, path AS List OF String, defaultValue AS Json) AS Json
  MUT current AS Json = value
  FOR EACH key IN path
    MUT nextValue AS Json = current
    LET currentValue AS Json = current
    MATCH currentValue
      CASE JsonObj(obj)
        IF collections::hasKey(obj.fields, key) THEN
          nextValue = collections::get(obj.fields, key)
        ELSE
          RETURN defaultValue
        END IF
      CASE ELSE
        RETURN defaultValue
    END MATCH
    current = nextValue
  NEXT
  RETURN current
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_GET_OR,
    return_type: ReturnType::Fixed("Json"),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const GET_OR: BuiltinFunction =
    BuiltinFunction::mfb("json.getOr", "getOr", INTRO, DESC, &[], OV, BODY);
