//! `__encoding_isAlphaNum` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_isAlphaNum(c AS Integer) AS Boolean
  IF c >= 65 AND c <= 90 THEN
    RETURN TRUE
  END IF
  IF c >= 97 AND c <= 122 THEN
    RETURN TRUE
  END IF
  IF c >= 48 AND c <= 57 THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_isAlphaNum", BODY));
}
