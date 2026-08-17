//! `__crypto_bsig1_64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_bsig1_64(x AS Integer) AS Integer
  LET a AS Integer = bits::rr64(x, 14)
  LET b AS Integer = bits::rr64(x, 18)
  LET c AS Integer = bits::rr64(x, 41)
  RETURN bits::bxor(bits::bxor(a, b), c)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_bsig1_64", BODY));
}
