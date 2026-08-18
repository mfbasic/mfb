//! `__crypto_chachaQr` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' One ChaCha20 quarter-round on state words a,b,c,d, mutating `s` in place.
FUNC __crypto_chachaQr(s AS List OF Integer, ai AS Integer, bi AS Integer, ci AS Integer, di AS Integer) AS List OF Integer
  MUT st AS List OF Integer = s
  MUT a AS Integer = collections::get(st, ai)
  MUT b AS Integer = collections::get(st, bi)
  MUT c AS Integer = collections::get(st, ci)
  MUT d AS Integer = collections::get(st, di)
  a = __crypto_add32(a, b)
  d = bits::rl32(bits::bxor(d, a), 16)
  c = __crypto_add32(c, d)
  b = bits::rl32(bits::bxor(b, c), 12)
  a = __crypto_add32(a, b)
  d = bits::rl32(bits::bxor(d, a), 8)
  c = __crypto_add32(c, d)
  b = bits::rl32(bits::bxor(b, c), 7)
  st = collections::set(st, ai, a)
  st = collections::set(st, bi, b)
  st = collections::set(st, ci, c)
  st = collections::set(st, di, d)
  RETURN st
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_chachaQr", BODY));
}
