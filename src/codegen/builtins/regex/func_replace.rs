//! `regex::replace` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:replace@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_replace(value AS String, pattern AS String, replacement AS String) AS String
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  MUT out AS String = ""
  MUT cursor AS Integer = 0
  FOR EACH r IN __regex_matchResults(prog, ctx, 0)
    LET mstart AS Integer = collections::get(r.caps, 0)
    out = out & strings::mid(value, cursor, mstart - cursor)
    out = out & __regex_expand(replacement, r, value, prog)
    cursor = r.pos
  NEXT
  out = out & strings::mid(value, cursor, ctx.n - cursor)
  RETURN out
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::PARAMS_REPLACE,
    return_type: ReturnType::Fixed("String"),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const REPLACE: BuiltinFunction =
    BuiltinFunction::mfb("regex.replace", "replace", INTRO, DESC, &[], OV, BODY);
