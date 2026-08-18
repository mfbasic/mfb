//! `__encoding_punyDigit` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Base-36 digit (0..35) to its Punycode character code (a-z then 0-9).
FUNC __encoding_punyDigit(d AS Integer) AS Integer
  IF d < 26 THEN
    RETURN d + 97
  END IF
  RETURN d - 26 + 48
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_punyDigit", BODY));
}
