//! `__crypto_gf0` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_gf0() AS List OF Integer
  MUT g AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    g = collections::append(g, 0)
    i = i + 1
  END WHILE
  RETURN g
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf0", BODY));
}
