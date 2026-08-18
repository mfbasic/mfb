//! `__crypto_cswap128` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Constant-time conditional swap of the two 64-limb points in `pair` when b=1.
FUNC __crypto_cswap128(pair AS List OF Integer, b AS Integer) AS List OF Integer
  LET mask AS Integer = 0 - b
  MUT r AS List OF Integer = pair
  MUT i AS Integer = 0
  WHILE i < 64
    LET x AS Integer = collections::get(r, i)
    LET y AS Integer = collections::get(r, 64 + i)
    LET t AS Integer = bits::band(mask, bits::bxor(x, y))
    r = collections::set(r, i, bits::bxor(x, t))
    r = collections::set(r, 64 + i, bits::bxor(y, t))
    i = i + 1
  END WHILE
  RETURN r
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_cswap128", BODY));
}
