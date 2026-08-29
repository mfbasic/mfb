//! `__crypto_gf448Pack` — shared private helper for the `crypto` package.
//!
//! Canonical 56-byte little-endian encoding: three carry passes bring the limbs
//! strictly below 2^28 (so the value is `< 2^448 < 2p`), then one conditional
//! subtraction of `p` — computed as a full borrow chain and applied with a
//! branch-free `__crypto_gf448Select` on the final borrow — leaves the unique
//! representative in `0..p`. Limb pairs then pack into 7-byte groups.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Canonical 56-byte little-endian encoding of a GF(2^448-2^224-1) element.
FUNC __crypto_gf448Pack(n AS List OF Integer) AS List OF Byte
  MUT t AS List OF Integer = __crypto_gf448Carry(__crypto_gf448Carry(__crypto_gf448Carry(n)))
  MUT m AS List OF Integer = []
  MUT borrow AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 16
    MUT pi AS Integer = 268435455
    IF i = 8 THEN
      pi = 268435454
    END IF
    LET d AS Integer = collections::get(t, i) - pi - borrow
    borrow = bits::band(bits::sra(d, 28), 1)
    m = collections::append(m, bits::band(d, 268435455))
    i = i + 1
  END WHILE
  LET keep AS Integer = 0 - borrow
  LET r AS List OF Integer = __crypto_gf448Select(m, t, keep)
  MUT out AS List OF Byte = []
  MUT g AS Integer = 0
  WHILE g < 8
    LET v AS Integer = bits::bor(collections::get(r, 2 * g), bits::sl(collections::get(r, 2 * g + 1), 28))
    MUT k AS Integer = 0
    WHILE k < 7
      out = collections::append(out, toByte(bits::band(bits::sr(v, k * 8), 255)))
      k = k + 1
    END WHILE
    g = g + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448Pack", BODY));
}
