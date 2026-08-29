//! `__crypto_ed448Cswap` — shared private helper for the `crypto` package.
//!
//! Constant-time conditional swap of the two 48-limb points packed in `pair`
//! (96 limbs) when `b = 1`: mask `−b`, XOR-swap every limb — the 448-lane twin
//! of `__crypto_cswap128`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Branch-free conditional swap of the two 48-limb points in `pair` when b = 1.
FUNC __crypto_ed448Cswap(pair AS List OF Integer, b AS Integer) AS List OF Integer
  LET mask AS Integer = 0 - b
  MUT r AS List OF Integer = pair
  MUT i AS Integer = 0
  WHILE i < 48
    LET x AS Integer = collections::get(r, i)
    LET y AS Integer = collections::get(r, 48 + i)
    LET t AS Integer = bits::band(mask, bits::bxor(x, y))
    r = collections::set(r, i, bits::bxor(x, t))
    r = collections::set(r, 48 + i, bits::bxor(y, t))
    i = i + 1
  END WHILE
  RETURN r
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Cswap", BODY));
}
