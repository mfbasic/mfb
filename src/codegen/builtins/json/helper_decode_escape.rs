//! `__json_decodeEscape` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-510: `code` is the byte after the backslash. Every escape letter is ASCII,
' so a byte >= 128 falls through to the FAIL like any other unknown escape.
FUNC __json_decodeEscape(code AS Integer) AS String
  IF code = 34 THEN
    RETURN "\""
  ELSEIF code = 92 THEN
    RETURN "\\"
  ELSEIF code = 47 THEN
    RETURN "/"
  ELSEIF code = 98 THEN
    RETURN "\u{8}"
  ELSEIF code = 102 THEN
    RETURN "\u{C}"
  ELSEIF code = 110 THEN
    RETURN "\n"
  ELSEIF code = 114 THEN
    RETURN "\r"
  ELSEIF code = 116 THEN
    RETURN "\t"
  END IF

  FAIL error(77050003, "invalid JSON format")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_decodeEscape", BODY));
}
