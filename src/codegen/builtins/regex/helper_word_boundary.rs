//! `__regex_wordBoundary` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_wordBoundary(pos AS Integer, ctx AS __regex_Ctx) AS Boolean
  MUT before AS Boolean = FALSE
  MUT after AS Boolean = FALSE
  IF pos > 0 THEN
    before = __regex_isWord(collections::get(ctx.cps, pos - 1))
  END IF
  IF pos < ctx.n THEN
    after = __regex_isWord(collections::get(ctx.cps, pos))
  END IF
  RETURN before <> after
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_wordBoundary", BODY));
}
