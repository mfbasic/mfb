//! `__json_codePointToString` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_codePointToString(value AS Integer) AS String
  ' Keep json's own guard so a malformed code point raises json's error (and
  ' message), not encoding's. Every caller already excludes the surrogate range
  ' (control chars 0..31, BMP scalars, and combined surrogate pairs >= 65536),
  ' so the valid path never reaches utf32Decode's surrogate FAIL.
  IF value < 0 THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  IF value > 1114111 THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  RETURN encoding::utf32Decode([value])
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_codePointToString", BODY));
}
