//! `__crypto_car25519` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_car25519(o AS List OF Integer) AS List OF Integer
  MUT r AS List OF Integer = o
  MUT i AS Integer = 0
  WHILE i < 16
    LET v AS Integer = collections::get(r, i) + 65536
    LET c AS Integer = bits::sra(v, 16)
    IF i < 15 THEN
      r = collections::set(r, i + 1, collections::get(r, i + 1) + c - 1)
    ELSE
      r = collections::set(r, 0, collections::get(r, 0) + 38 * (c - 1))
    END IF
    r = collections::set(r, i, v - c * 65536)
    i = i + 1
  END WHILE
  RETURN r
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_car25519", BODY));
}
