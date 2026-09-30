//! `__regex_searchFrom` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_searchFrom(prog AS __regex_Program, ctx AS __regex_Ctx, from AS Integer) AS __regex_Result
  ' bug-315: one budget per search, not per start offset -- the scan over start
  ' positions is itself part of the work an adversarial pattern multiplies.
  __regex_steps = 0
  ' bug-773: __regex_run scans the start positions itself, skipping those that cannot
  ' begin a match (plan-77 R5's fixed first literal, or the first-scalar set), so one
  ' set of engine tables serves the whole search.
  RETURN __regex_run(prog, prog.leaves, from, ctx)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_searchFrom", BODY));
}
