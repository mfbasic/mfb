//! `__encoding_hexValue` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_hexValue(c AS Integer) AS Integer
  IF c >= 48 AND c <= 57 THEN
    RETURN c - 48
  END IF
  IF c >= 97 AND c <= 102 THEN
    RETURN c - 87
  END IF
  IF c >= 65 AND c <= 70 THEN
    RETURN c - 55
  END IF
  RETURN -1
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_hexValue", BODY));
}
