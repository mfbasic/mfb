//! `__http_decToInt` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse an unsigned status-line decimal field. Same shape as the hex parser:
' reject a sign, delegate to toInt(text, 10), keep http's status messages.
FUNC __http_decToInt(text AS String) AS Integer
  IF text = "" THEN
    FAIL error(77050003, "invalid status line")
  END IF
  IF strings::startsWith(text, "-") OR strings::startsWith(text, "+") THEN
    FAIL error(77050003, "invalid status code")
  END IF
  LET value AS Integer = toInt(text, 10) TRAP(err)
    FAIL error(77050003, "invalid status code")
  END TRAP
  RETURN value
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_decToInt", BODY));
}
