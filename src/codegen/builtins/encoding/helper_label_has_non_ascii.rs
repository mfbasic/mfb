//! `__encoding_labelHasNonAscii` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_labelHasNonAscii(points AS List OF Integer) AS Boolean
  FOR EACH cp IN points
    IF cp >= 128 THEN
      RETURN TRUE
    END IF
  NEXT
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_labelHasNonAscii", BODY));
}
