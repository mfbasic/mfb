//! `__encoding_base32Value` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base32Value(c AS Integer) AS Integer
  IF c >= 65 AND c <= 90 THEN
    RETURN c - 65
  END IF
  IF c >= 97 AND c <= 122 THEN
    RETURN c - 97
  END IF
  IF c >= 50 AND c <= 55 THEN
    RETURN c - 24
  END IF
  RETURN -1
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_base32Value", BODY));
}
