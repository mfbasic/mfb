//! `__json_parseHexQuad` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseHexQuad(chars AS List OF String, index AS Integer) AS Integer
  IF index + 3 >= len(chars) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  ' Strict 4-digit `\uXXXX` escape: concatenate exactly four chars and let
  ' toInt(_, 16) validate the hex digits, keeping json's own error.
  LET quad AS String = collections::get(chars, index) & collections::get(chars, index + 1) & collections::get(chars, index + 2) & collections::get(chars, index + 3)
  LET value AS Integer = toInt(quad, 16) TRAP(err)
    FAIL error(77050003, "invalid JSON format")
  END TRAP
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseHexQuad", BODY));
}
