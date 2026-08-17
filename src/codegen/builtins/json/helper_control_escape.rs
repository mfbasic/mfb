//! `__json_controlEscape` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_controlEscape(codePoint AS Integer) AS String
  IF codePoint = 8 THEN
    RETURN "\\b"
  ELSEIF codePoint = 9 THEN
    RETURN "\\t"
  ELSEIF codePoint = 10 THEN
    RETURN "\\n"
  ELSEIF codePoint = 12 THEN
    RETURN "\\f"
  ELSEIF codePoint = 13 THEN
    RETURN "\\r"
  END IF
  RETURN __json_unicodeControlEscape(codePoint)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_controlEscape", BODY));
}
