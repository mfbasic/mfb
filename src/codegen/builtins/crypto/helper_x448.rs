//! `__crypto_x448` — shared private helper for the `crypto` package.
//!
//! The X448 (Curve448 ECDH) scalar multiplication of RFC 7748 §5: the Montgomery
//! ladder `X448(scalar, u) -> 56 bytes` with `a24 = 39081`, exactly 448 fixed
//! iterations from bit 447 down, over the `__crypto_gf448*` field. The scalar is
//! clamped internally (`decodeScalar448`), so a raw 56-byte scalar and an
//! already-clamped key both give the RFC result. The conditional swap is the
//! RFC's deferred `swap ^= k_t` form, realised as two branch-free
//! `__crypto_gf448Select`s under an all-ones/zero mask — no control flow depends
//! on the scalar. The result is canonically encoded; a low-order `u` yields the
//! all-zero output, which the callers (`crypto::exchange`, the HPKE KEM) reject.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 7748 §5 X448 Montgomery ladder over GF(2^448-2^224-1). `scalar`/`point` are
' 56-byte little-endian; returns the 56-byte little-endian shared u-coordinate.
FUNC __crypto_x448(scalar AS List OF Byte, point AS List OF Byte) AS List OF Byte
  LET k AS List OF Byte = __crypto_clampScalar448(scalar)
  LET x1 AS List OF Integer = __crypto_gf448Unpack(point)
  MUT x2 AS List OF Integer = __crypto_gf448One()
  MUT z2 AS List OF Integer = __crypto_gf448Zero()
  MUT x3 AS List OF Integer = x1
  MUT z3 AS List OF Integer = __crypto_gf448One()
  MUT swap AS Integer = 0
  MUT t AS Integer = 447
  WHILE t >= 0
    LET kt AS Integer = bits::band(bits::sr(toInt(collections::get(k, bits::sr(t, 3))), bits::band(t, 7)), 1)
    swap = bits::bxor(swap, kt)
    LET mask AS Integer = 0 - swap
    LET sx2 AS List OF Integer = __crypto_gf448Select(x2, x3, mask)
    x3 = __crypto_gf448Select(x3, x2, mask)
    x2 = sx2
    LET sz2 AS List OF Integer = __crypto_gf448Select(z2, z3, mask)
    z3 = __crypto_gf448Select(z3, z2, mask)
    z2 = sz2
    swap = kt
    LET a AS List OF Integer = __crypto_gf448Add(x2, z2)
    LET aa AS List OF Integer = __crypto_gf448Mul(a, a)
    LET b AS List OF Integer = __crypto_gf448Sub(x2, z2)
    LET bb AS List OF Integer = __crypto_gf448Mul(b, b)
    LET e AS List OF Integer = __crypto_gf448Sub(aa, bb)
    LET c AS List OF Integer = __crypto_gf448Add(x3, z3)
    LET d AS List OF Integer = __crypto_gf448Sub(x3, z3)
    LET da AS List OF Integer = __crypto_gf448Mul(d, a)
    LET cb AS List OF Integer = __crypto_gf448Mul(c, b)
    LET s AS List OF Integer = __crypto_gf448Add(da, cb)
    x3 = __crypto_gf448Mul(s, s)
    LET f AS List OF Integer = __crypto_gf448Sub(da, cb)
    z3 = __crypto_gf448Mul(x1, __crypto_gf448Mul(f, f))
    x2 = __crypto_gf448Mul(aa, bb)
    z2 = __crypto_gf448Mul(e, __crypto_gf448Add(aa, __crypto_gf448MulSmall(e, 39081)))
    t = t - 1
  END WHILE
  LET fmask AS Integer = 0 - swap
  LET rx AS List OF Integer = __crypto_gf448Select(x2, x3, fmask)
  LET rz AS List OF Integer = __crypto_gf448Select(z2, z3, fmask)
  RETURN __crypto_gf448Pack(__crypto_gf448Mul(rx, __crypto_gf448Inv(rz)))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_x448", BODY));
}
