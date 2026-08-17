//! `__crypto_gfD` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_gfD() AS List OF Integer
  MUT g AS List OF Integer = []
  g = collections::append(g, 30883)
  g = collections::append(g, 4953)
  g = collections::append(g, 19914)
  g = collections::append(g, 30187)
  g = collections::append(g, 55467)
  g = collections::append(g, 16705)
  g = collections::append(g, 2637)
  g = collections::append(g, 112)
  g = collections::append(g, 59544)
  g = collections::append(g, 30585)
  g = collections::append(g, 16505)
  g = collections::append(g, 36039)
  g = collections::append(g, 65139)
  g = collections::append(g, 11119)
  g = collections::append(g, 27886)
  g = collections::append(g, 20995)
  RETURN g
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gfD", BODY));
}
