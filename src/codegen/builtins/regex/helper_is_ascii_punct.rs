//! `__regex_isAsciiPunct` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_isAsciiPunct(ch AS String) AS Boolean
  IF ch >= "!" AND ch <= "/" THEN
    RETURN TRUE
  END IF
  IF ch >= ":" AND ch <= "@" THEN
    RETURN TRUE
  END IF
  IF ch >= "[" AND ch <= "`" THEN
    RETURN TRUE
  END IF
  IF ch >= "{" AND ch <= "~" THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_isAsciiPunct", BODY));
}
