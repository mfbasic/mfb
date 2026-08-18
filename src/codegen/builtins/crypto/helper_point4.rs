//! `__crypto_point4` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_point4(x AS List OF Integer, y AS List OF Integer, z AS List OF Integer, t AS List OF Integer) AS List OF Integer
  MUT p AS List OF Integer = __crypto_concatInt(x, y)
  p = __crypto_concatInt(p, z)
  p = __crypto_concatInt(p, t)
  RETURN p
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_point4", BODY));
}
