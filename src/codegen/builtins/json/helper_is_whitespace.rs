//! `__json_isWhitespace` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_isWhitespace(ch AS String) AS Boolean
  IF ch = " " THEN
    RETURN TRUE
  END IF
  IF ch = "\t" THEN
    RETURN TRUE
  END IF
  IF ch = "\n" THEN
    RETURN TRUE
  END IF
  IF ch = "\r" THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_isWhitespace", BODY));
}
