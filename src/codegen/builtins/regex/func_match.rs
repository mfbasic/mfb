//! `regex::match` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:match@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_match(value AS String, pattern AS String) AS Boolean
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  LET r AS __regex_Result = __regex_searchFrom(prog, ctx, 0)
  RETURN r.ok
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::PARAMS_MATCH,
    return_type: ReturnType::Fixed("Boolean"),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const MATCH: BuiltinFunction =
    BuiltinFunction::mfb("regex.match", "match", INTRO, DESC, &[], OV, BODY);
