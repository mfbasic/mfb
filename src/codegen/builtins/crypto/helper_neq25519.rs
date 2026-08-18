//! `__crypto_neq25519` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_neq25519(a AS List OF Integer, b AS List OF Integer) AS Boolean
  LET pa AS List OF Byte = __crypto_pack25519(a)
  LET pb AS List OF Byte = __crypto_pack25519(b)
  RETURN __crypto_constantTimeEqual(pa, pb) = FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_neq25519", BODY));
}
