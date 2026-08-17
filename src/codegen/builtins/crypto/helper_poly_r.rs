//! `__crypto_polyR` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Clamp the 16-byte `r` half of the one-time key into five 26-bit limbs.
FUNC __crypto_polyR(key AS List OF Byte) AS List OF Integer
  LET t0 AS Integer = __crypto_leWord(key, 0)
  LET t1 AS Integer = __crypto_leWord(key, 4)
  LET t2 AS Integer = __crypto_leWord(key, 8)
  LET t3 AS Integer = __crypto_leWord(key, 12)
  MUT r AS List OF Integer = []
  LET r0 AS Integer = bits::band(t0, 67108863)
  LET a1 AS Integer = bits::bor(bits::sr(t0, 26), bits::sl(t1, 6))
  LET r1 AS Integer = bits::band(a1, 67108611)
  LET a2 AS Integer = bits::bor(bits::sr(t1, 20), bits::sl(t2, 12))
  LET r2 AS Integer = bits::band(a2, 67092735)
  LET a3 AS Integer = bits::bor(bits::sr(t2, 14), bits::sl(t3, 18))
  LET r3 AS Integer = bits::band(a3, 66076671)
  LET r4 AS Integer = bits::band(bits::sr(t3, 8), 1048575)
  r = collections::append(r, r0)
  r = collections::append(r, r1)
  r = collections::append(r, r2)
  r = collections::append(r, r3)
  r = collections::append(r, r4)
  RETURN r
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_polyR", BODY));
}
