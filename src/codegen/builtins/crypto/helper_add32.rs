//! `__crypto_add32` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' 32-bit modular addition. `a` and `b` must already be in 0..2^32-1.
FUNC __crypto_add32(a AS Integer, b AS Integer) AS Integer
  LET s AS Integer = a + b
  RETURN bits::band(s, 4294967295)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_add32", BODY));
}
