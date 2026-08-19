//! `__crypto_gf121665` — shared private helper for the `crypto` package.
//!
//! The Curve25519 Montgomery-ladder constant `a24 = (486662 - 2) / 4 = 121665`,
//! encoded as a `gf` (radix-2^16 field element): `121665 = 0xDB41 + 1·2^16`, so
//! limb0 = 0xDB41 = 56129 and limb1 = 1. Used by `__crypto_x25519`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_gf121665() AS List OF Integer
  MUT g AS List OF Integer = __crypto_gf0()
  g = collections::set(g, 0, 56129)
  g = collections::set(g, 1, 1)
  RETURN g
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf121665", BODY));
}
