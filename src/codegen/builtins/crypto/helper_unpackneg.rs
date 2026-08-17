//! `__crypto_unpackneg` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Decode a compressed public key to the negated extended-coordinate point.
' Returns a 65-element list: element 0 = 1 when valid (else 0), 1..64 = point.
FUNC __crypto_unpackneg(p AS List OF Byte) AS List OF Integer
  LET r2 AS List OF Integer = __crypto_gf1()
  LET r1 AS List OF Integer = __crypto_unpack25519(p)
  LET num0 AS List OF Integer = __crypto_edS(r1)
  MUT den AS List OF Integer = __crypto_edM(num0, __crypto_gfD())
  LET num AS List OF Integer = __crypto_edZ(num0, r2)
  den = __crypto_edA(r2, den)
  LET den2 AS List OF Integer = __crypto_edS(den)
  LET den4 AS List OF Integer = __crypto_edS(den2)
  LET den6 AS List OF Integer = __crypto_edM(den4, den2)
  MUT t AS List OF Integer = __crypto_edM(den6, num)
  t = __crypto_edM(t, den)
  t = __crypto_pow2523(t)
  t = __crypto_edM(t, num)
  t = __crypto_edM(t, den)
  t = __crypto_edM(t, den)
  MUT r0 AS List OF Integer = __crypto_edM(t, den)
  LET chk1 AS List OF Integer = __crypto_edM(__crypto_edS(r0), den)
  IF __crypto_neq25519(chk1, num) THEN
    r0 = __crypto_edM(r0, __crypto_gfI())
  END IF
  LET chk2 AS List OF Integer = __crypto_edM(__crypto_edS(r0), den)
  IF __crypto_neq25519(chk2, num) THEN
    MUT bad AS List OF Integer = []
    bad = collections::append(bad, 0)
    MUT z AS Integer = 0
    WHILE z < 64
      bad = collections::append(bad, 0)
      z = z + 1
    END WHILE
    RETURN bad
  END IF
  LET par AS Integer = __crypto_par25519(r0)
  LET signBit AS Integer = bits::band(bits::sr(toInt(collections::get(p, 31)), 7), 1)
  IF par = signBit THEN
    r0 = __crypto_edZ(__crypto_gf0(), r0)
  END IF
  LET r3 AS List OF Integer = __crypto_edM(r0, r1)
  LET point AS List OF Integer = __crypto_point4(r0, r1, r2, r3)
  MUT ok AS List OF Integer = []
  ok = collections::append(ok, 1)
  RETURN __crypto_concatInt(ok, point)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_unpackneg", BODY));
}
