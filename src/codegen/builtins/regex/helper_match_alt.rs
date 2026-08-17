//! `__regex_matchAlt` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_matchAlt(opts AS List OF __regex_Node, i AS Integer, pos AS Integer, caps AS List OF Integer, c AS __regex_Cont, ctx AS __regex_Ctx, depth AS Integer) AS __regex_Result
  IF i >= len(opts) THEN
    RETURN __regex_fail()
  END IF
  LET r AS __regex_Result = __regex_matchNode(collections::get(opts, i), pos, caps, c, ctx, depth + 1)
  IF r.ok THEN
    RETURN r
  END IF
  RETURN __regex_matchAlt(opts, i + 1, pos, caps, c, ctx, depth + 1)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_matchAlt", BODY));
}
