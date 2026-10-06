//! `__http_isHexDigitChar` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-981: TRUE if `ch` is a single ASCII hex digit (0-9, a-f, A-F). Used by
' `__http_percentEncodeTarget` to recognize an already-present `%XX` escape so
' it is kept verbatim rather than double-encoded (its leading `%` re-escaped
' to `%25`).
FUNC __http_isHexDigitChar(ch AS String) AS Boolean
  IF len(ch) <> 1 THEN
    RETURN FALSE
  END IF
  RETURN strings::contains("0123456789abcdefABCDEF", ch)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_isHexDigitChar", BODY));
}
