//! `__json_unicodeControlEscape` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_unicodeControlEscape(codePoint AS Integer) AS String
  LET high AS Integer = codePoint / 16
  LET low AS Integer = codePoint - high * 16
  RETURN "\\u00" & __json_hexDigit(high) & __json_hexDigit(low)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_unicodeControlEscape", BODY));
}
