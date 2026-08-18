//! `__crypto_sha256Iv` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sha256Iv() AS List OF Integer
  MUT iv AS List OF Integer = []
  iv = collections::append(iv, 1779033703)
  iv = collections::append(iv, 3144134277)
  iv = collections::append(iv, 1013904242)
  iv = collections::append(iv, 2773480762)
  iv = collections::append(iv, 1359893119)
  iv = collections::append(iv, 2600822924)
  iv = collections::append(iv, 528734635)
  iv = collections::append(iv, 1541459225)
  RETURN iv
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha256Iv", BODY));
}
