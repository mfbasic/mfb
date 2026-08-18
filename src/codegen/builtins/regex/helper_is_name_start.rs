//! `__regex_isNameStart` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_isNameStart(ch AS String) AS Boolean
  IF ch >= "A" AND ch <= "Z" THEN
    RETURN TRUE
  END IF
  IF ch >= "a" AND ch <= "z" THEN
    RETURN TRUE
  END IF
  RETURN ch = "_"
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_isNameStart", BODY));
}
