//! `__encoding_punyThreshold` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_punyThreshold(k AS Integer, bias AS Integer) AS Integer
  IF k <= bias + 1 THEN
    RETURN 1
  END IF
  IF k >= bias + 26 THEN
    RETURN 26
  END IF
  RETURN k - bias
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_punyThreshold", BODY));
}
