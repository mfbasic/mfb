//! `__crypto_gfD2` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_gfD2() AS List OF Integer
  MUT g AS List OF Integer = []
  g = collections::append(g, 61785)
  g = collections::append(g, 9906)
  g = collections::append(g, 39828)
  g = collections::append(g, 60374)
  g = collections::append(g, 45398)
  g = collections::append(g, 33411)
  g = collections::append(g, 5274)
  g = collections::append(g, 224)
  g = collections::append(g, 53552)
  g = collections::append(g, 61171)
  g = collections::append(g, 33010)
  g = collections::append(g, 6542)
  g = collections::append(g, 64743)
  g = collections::append(g, 22239)
  g = collections::append(g, 55772)
  g = collections::append(g, 9222)
  RETURN g
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gfD2", BODY));
}
