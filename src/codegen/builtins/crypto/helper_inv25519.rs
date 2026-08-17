//! `__crypto_inv25519` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_inv25519(i0 AS List OF Integer) AS List OF Integer
  MUT c AS List OF Integer = i0
  MUT a AS Integer = 253
  WHILE a >= 0
    c = __crypto_edS(c)
    IF a <> 2 AND a <> 4 THEN
      c = __crypto_edM(c, i0)
    END IF
    a = a - 1
  END WHILE
  RETURN c
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_inv25519", BODY));
}
