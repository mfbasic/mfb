//! `__crypto_bsig1` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_bsig1(x AS Integer) AS Integer
  LET a AS Integer = __crypto_rotr32(x, 6)
  LET b AS Integer = __crypto_rotr32(x, 11)
  LET c AS Integer = __crypto_rotr32(x, 25)
  RETURN bits::bxor(bits::bxor(a, b), c)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_bsig1", BODY));
}
