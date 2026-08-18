//! `__crypto_sha224Iv` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sha224Iv() AS List OF Integer
  MUT iv AS List OF Integer = []
  iv = collections::append(iv, 3238371032)
  iv = collections::append(iv, 914150663)
  iv = collections::append(iv, 812702999)
  iv = collections::append(iv, 4144912697)
  iv = collections::append(iv, 4290775857)
  iv = collections::append(iv, 1750603025)
  iv = collections::append(iv, 1694076839)
  iv = collections::append(iv, 3204075428)
  RETURN iv
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha224Iv", BODY));
}
