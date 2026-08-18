//! `__json_decodeEscape` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_decodeEscape(ch AS String) AS String
  IF ch = "\"" THEN
    RETURN "\""
  ELSEIF ch = "\\" THEN
    RETURN "\\"
  ELSEIF ch = "/" THEN
    RETURN "/"
  ELSEIF ch = "b" THEN
    RETURN "\u{8}"
  ELSEIF ch = "f" THEN
    RETURN "\u{C}"
  ELSEIF ch = "n" THEN
    RETURN "\n"
  ELSEIF ch = "r" THEN
    RETURN "\r"
  ELSEIF ch = "t" THEN
    RETURN "\t"
  END IF

  FAIL error(77050003, "invalid JSON format")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_decodeEscape", BODY));
}
