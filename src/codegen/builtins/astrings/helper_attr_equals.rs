//! `__astrings_attrEquals` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Structural equality of a stored span's attribute vs a target span's attribute
REM (class + member + payload). Used by removeAttribute to match spans to drop.
FUNC __astrings_attrEquals(a AS AttrSpan, b AS AttrSpan) AS Boolean
  IF a.class <> b.class THEN
    RETURN FALSE
  END IF
  IF a.member <> b.member THEN
    RETURN FALSE
  END IF
  IF a.class = 1 THEN
    RETURN a.text = b.text
  END IF
  IF a.class = 2 THEN
    RETURN a.number = b.number
  END IF
  RETURN TRUE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_attrEquals", BODY));
}
