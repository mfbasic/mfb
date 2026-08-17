//! `__regex_allDigits` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_allDigits(s AS String) AS Boolean
  IF len(s) = 0 THEN
    RETURN FALSE
  END IF
  FOR EACH ch IN __regex_toScalars(s)
    IF __regex_isDigit(ch) = FALSE THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_allDigits", BODY));
}
