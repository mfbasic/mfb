//! `__crypto_x25519` — shared private helper for the `crypto` package.
//!
//! The X25519 (Curve25519 ECDH) scalar multiplication of RFC 7748 §5: the
//! Montgomery ladder `X25519(scalar, u) -> 32 bytes`, computed over the SAME
//! GF(2^255-19) field arithmetic the Ed25519 core already provides
//! (`__crypto_edA`/`edZ`/`edS`/`edM`/`car25519`/`inv25519`, `__crypto_unpack25519`
//! /`pack25519`). The scalar is clamped internally per RFC 7748 (`k[0] &= 248;
//! k[31] &= 127; k[31] |= 64`), so a raw 32-byte scalar (the §5.2 KAT input) and
//! an already-clamped keypair scalar both give the RFC result. The conditional
//! swap of the `(a,b)`/`(c,d)` gf pairs is a plain branch on the key bit — the
//! output field values are identical to the constant-time masked form.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 7748 §5 Montgomery ladder over GF(2^255-19). `scalar`/`point` are 32-byte
' little-endian; returns the 32-byte little-endian shared u-coordinate.
FUNC __crypto_x25519(scalar AS List OF Byte, point AS List OF Byte) AS List OF Byte
  LET z AS List OF Byte = __crypto_clampScalar(scalar)
  LET x AS List OF Integer = __crypto_unpack25519(point)
  MUT a AS List OF Integer = __crypto_gf1()
  MUT b AS List OF Integer = x
  MUT c AS List OF Integer = __crypto_gf0()
  MUT d AS List OF Integer = __crypto_gf1()
  MUT e AS List OF Integer = __crypto_gf0()
  MUT f AS List OF Integer = __crypto_gf0()
  MUT i AS Integer = 254
  WHILE i >= 0
    LET byteIdx AS Integer = bits::sr(i, 3)
    LET bitIdx AS Integer = bits::band(i, 7)
    LET r AS Integer = bits::band(bits::sr(toInt(collections::get(z, byteIdx)), bitIdx), 1)
    IF r = 1 THEN
      LET ta AS List OF Integer = a
      a = b
      b = ta
      LET tc AS List OF Integer = c
      c = d
      d = tc
    END IF
    e = __crypto_edA(a, c)
    a = __crypto_edZ(a, c)
    c = __crypto_edA(b, d)
    b = __crypto_edZ(b, d)
    d = __crypto_edS(e)
    f = __crypto_edS(a)
    a = __crypto_edM(c, a)
    c = __crypto_edM(b, e)
    e = __crypto_edA(a, c)
    a = __crypto_edZ(a, c)
    b = __crypto_edS(a)
    c = __crypto_edZ(d, f)
    a = __crypto_edM(c, __crypto_gf121665())
    a = __crypto_edA(a, d)
    c = __crypto_edM(c, a)
    a = __crypto_edM(d, f)
    d = __crypto_edM(b, x)
    b = __crypto_edS(e)
    IF r = 1 THEN
      LET ua AS List OF Integer = a
      a = b
      b = ua
      LET uc AS List OF Integer = c
      c = d
      d = uc
    END IF
    i = i - 1
  END WHILE
  c = __crypto_inv25519(c)
  a = __crypto_edM(a, c)
  RETURN __crypto_pack25519(a)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_x25519", BODY));
}
