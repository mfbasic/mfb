//! `__crypto_gfI` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_gfI() AS List OF Integer
  MUT g AS List OF Integer = []
  g = collections::append(g, 41136)
  g = collections::append(g, 18958)
  g = collections::append(g, 6951)
  g = collections::append(g, 50414)
  g = collections::append(g, 58488)
  g = collections::append(g, 44335)
  g = collections::append(g, 6150)
  g = collections::append(g, 12099)
  g = collections::append(g, 55207)
  g = collections::append(g, 15867)
  g = collections::append(g, 153)
  g = collections::append(g, 11085)
  g = collections::append(g, 57099)
  g = collections::append(g, 20417)
  g = collections::append(g, 9344)
  g = collections::append(g, 11139)
  RETURN g
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gfI", BODY));
}
