//! `__crypto_maj32` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_maj32(x AS Integer, y AS Integer, z AS Integer) AS Integer
  LET a AS Integer = bits::band(x, y)
  LET b AS Integer = bits::band(x, z)
  LET c AS Integer = bits::band(y, z)
  RETURN bits::bxor(bits::bxor(a, b), c)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_maj32", BODY));
}
