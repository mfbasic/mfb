//! `__crypto_gf448MulSmall` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GF(2^448-2^224-1) multiplication by a small public constant k (< 2^20).
FUNC __crypto_gf448MulSmall(a AS List OF Integer, k AS Integer) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    o = collections::append(o, collections::get(a, i) * k)
    i = i + 1
  END WHILE
  RETURN __crypto_gf448Carry(__crypto_gf448Carry(o))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448MulSmall", BODY));
}
