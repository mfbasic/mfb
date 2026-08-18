//! `__encoding_base64Value` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_base64Value(c AS Integer, urlSafe AS Boolean) AS Integer
  IF c >= 65 AND c <= 90 THEN
    RETURN c - 65
  END IF
  IF c >= 97 AND c <= 122 THEN
    RETURN c - 71
  END IF
  IF c >= 48 AND c <= 57 THEN
    RETURN c + 4
  END IF
  IF urlSafe THEN
    IF c = 45 THEN
      RETURN 62
    END IF
    IF c = 95 THEN
      RETURN 63
    END IF
  ELSE
    IF c = 43 THEN
      RETURN 62
    END IF
    IF c = 47 THEN
      RETURN 63
    END IF
  END IF
  RETURN -1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_base64Value", BODY));
}
