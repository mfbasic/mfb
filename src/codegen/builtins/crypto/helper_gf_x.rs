//! `__crypto_gfX` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_gfX() AS List OF Integer
  MUT g AS List OF Integer = []
  g = collections::append(g, 54554)
  g = collections::append(g, 36645)
  g = collections::append(g, 11616)
  g = collections::append(g, 51542)
  g = collections::append(g, 42930)
  g = collections::append(g, 38181)
  g = collections::append(g, 51040)
  g = collections::append(g, 26924)
  g = collections::append(g, 56412)
  g = collections::append(g, 64982)
  g = collections::append(g, 57905)
  g = collections::append(g, 49316)
  g = collections::append(g, 21502)
  g = collections::append(g, 52590)
  g = collections::append(g, 14035)
  g = collections::append(g, 8553)
  RETURN g
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gfX", BODY));
}
