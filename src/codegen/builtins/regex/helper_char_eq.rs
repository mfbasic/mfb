//! `__regex_charEq` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_charEq(a AS String, b AS String, fold AS Boolean) AS Boolean
  IF fold THEN
    RETURN strings::caseFold(a) = strings::caseFold(b)
  END IF
  RETURN a = b
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_charEq", BODY));
}
