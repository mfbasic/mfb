//! `__json_isWhitespace` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' JSON whitespace is exactly space, tab, line feed and carriage return -- four
' ASCII bytes, so `code` is the byte under the scanner (bug-510).
FUNC __json_isWhitespace(code AS Integer) AS Boolean
  IF code = 32 THEN
    RETURN TRUE
  END IF
  IF code = 9 THEN
    RETURN TRUE
  END IF
  IF code = 10 THEN
    RETURN TRUE
  END IF
  IF code = 13 THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_isWhitespace", BODY));
}
