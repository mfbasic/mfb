//! `__json_parseEscape` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseEscape(chars AS List OF String, index AS Integer) AS __json_StringNode
  IF index >= len(chars) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  LET ch AS String = collections::get(chars, index)
  IF ch = "u" THEN
    RETURN __json_parseUnicodeEscape(chars, index + 1)
  END IF
  RETURN __json_StringNode[__json_decodeEscape(ch), index + 1]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseEscape", BODY));
}
