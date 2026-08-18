//! `__crypto_ch64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ch64(x AS Integer, y AS Integer, z AS Integer) AS Integer
  LET l AS Integer = bits::band(x, y)
  LET r AS Integer = bits::band(bits::bnot(x), z)
  RETURN bits::bxor(l, r)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ch64", BODY));
}
