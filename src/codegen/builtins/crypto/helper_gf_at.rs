//! `__crypto_gfAt` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_gfAt(p AS List OF Integer, which AS Integer) AS List OF Integer
  MUT g AS List OF Integer = []
  LET base AS Integer = which * 16
  MUT i AS Integer = 0
  WHILE i < 16
    g = collections::append(g, collections::get(p, base + i))
    i = i + 1
  END WHILE
  RETURN g
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gfAt", BODY));
}
