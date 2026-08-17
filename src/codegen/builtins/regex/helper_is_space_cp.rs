//! `__regex_isSpaceCp` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_isSpaceCp(cp AS Integer, cat AS String) AS Boolean
  IF strings::startsWith(cat, "Z") THEN
    RETURN TRUE
  END IF
  IF cp >= 9 AND cp <= 13 THEN
    RETURN TRUE
  END IF
  IF cp = 133 THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_isSpaceCp", BODY));
}
