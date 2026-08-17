//! `__encoding_percentByte` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_percentByte(c AS Integer) AS String
  RETURN "%" & strings::upper(__encoding_hexDigit(c / 16) & __encoding_hexDigit(c - (c / 16) * 16))
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_percentByte", BODY));
}
