//! `__encoding_punyValue` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Punycode character code to its base-36 digit, or -1 when invalid.
FUNC __encoding_punyValue(c AS Integer) AS Integer
  IF c >= 97 AND c <= 122 THEN
    RETURN c - 97
  END IF
  IF c >= 65 AND c <= 90 THEN
    RETURN c - 65
  END IF
  IF c >= 48 AND c <= 57 THEN
    RETURN c - 22
  END IF
  RETURN -1
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_punyValue", BODY));
}
