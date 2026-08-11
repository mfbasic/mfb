//! `regex::find` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:find@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_find(value AS String, pattern AS String, start AS Integer) AS Integer
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  IF start < 0 OR start > ctx.n THEN
    FAIL error(77050001, "List or string index/range is outside valid bounds.")
  END IF
  LET r AS __regex_Result = __regex_searchFrom(prog, ctx, start)
  IF r.ok = FALSE THEN
    RETURN -1
  END IF
  RETURN collections::get(r.caps, 0)
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::PARAMS_FIND,
    return_type: ReturnType::Fixed("Integer"),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const FIND: BuiltinFunction =
    BuiltinFunction::mfb("regex.find", "find", INTRO, DESC, &[], OV, BODY);
