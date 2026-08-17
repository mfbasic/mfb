//! `__regex_tryAt` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_tryAt(prog AS __regex_Program, ctx AS __regex_Ctx, start AS Integer) AS __regex_Result
  MUT caps AS List OF Integer = __regex_initCaps(prog.groups)
  caps = __regex_setCap(caps, 0, start)
  ' bug-315: depth 0 seeds the recursion-depth guard threaded through the matcher.
  RETURN __regex_matchNode(prog.root, start, caps, __regex_ContCap[0, __regex_ContDone[TRUE]], ctx, 0)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_tryAt", BODY));
}
