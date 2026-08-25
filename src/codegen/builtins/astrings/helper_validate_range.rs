//! `__astrings_validateRange` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM Validate an inclusive scalar range against the visible scalar count. An op on
REM an empty AttributedString always errors (n = 0 → endIndex >= 0 fails). Returns the
REM scalar count.
FUNC __astrings_validateRange(a AS AttributedString, start AS Integer, endIndex AS Integer) AS Integer
  LET n AS Integer = astrings::scalarLen(a)
  IF start < 0 OR endIndex < start THEN
    FAIL error(77050002, "invalid attribute range")
  END IF
  IF endIndex >= n THEN
    FAIL error(77050001, "attribute range out of bounds")
  END IF
  RETURN n
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_validateRange", BODY));
}
