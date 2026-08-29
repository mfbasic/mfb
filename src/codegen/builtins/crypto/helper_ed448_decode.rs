//! `__crypto_ed448Decode` — shared private helper for the `crypto` package.
//!
//! Strict RFC 8032 §5.2.3 point decoding of a 57-byte encoding, returning a
//! 49-element list: element 0 is 1 when valid (else 0) and elements 1..48 the
//! projective point. Rejects, in order: a wrong length; any of the seven unused
//! bits of the sign byte set; a non-canonical `y` (`y ≥ p`, detected because the
//! canonical re-encoding differs); a `y` with no square root `x` (not on the
//! curve); `x = 0` with the sign bit set; and finally a point of small order —
//! the 4-torsion subgroup `{(0, ±1), (±1, 0)}`, exactly the points with `x = 0`
//! or `y = 0` — so a public key or `R` that would make the verification equation
//! trivially true is refused (libsodium's rule; RFC 8032 leaves it optional).
//! The square root is `x = u³·v·(u⁵·v³)^((p−3)/4)` with `u = y² − 1`,
//! `v = d·y² − 1`, checked by `v·x² = u`, and negated to match the sign bit.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Strictly decode a 57-byte edwards448 point: [ok] + 48 limbs (ok = 1 when valid).
FUNC __crypto_ed448Decode(b AS List OF Byte) AS List OF Integer
  IF len(b) <> 57 THEN
    RETURN __crypto_zeroLimbs(49)
  END IF
  IF bits::band(toInt(collections::get(b, 56)), 127) <> 0 THEN
    RETURN __crypto_zeroLimbs(49)
  END IF
  LET yb AS List OF Byte = collections::mid(b, 0, 56)
  LET y AS List OF Integer = __crypto_gf448Unpack(yb)
  IF __crypto_constantTimeEqual(__crypto_gf448Pack(y), yb) = FALSE THEN
    RETURN __crypto_zeroLimbs(49)
  END IF
  LET x0 AS Integer = bits::sr(toInt(collections::get(b, 56)), 7)
  LET zero AS List OF Integer = __crypto_gf448Zero()
  LET one AS List OF Integer = __crypto_gf448One()
  LET y2 AS List OF Integer = __crypto_gf448Mul(y, y)
  LET u AS List OF Integer = __crypto_gf448Sub(y2, one)
  LET v AS List OF Integer = __crypto_gf448Sub(__crypto_gf448Sub(zero, __crypto_gf448MulSmall(y2, 39081)), one)
  LET u2 AS List OF Integer = __crypto_gf448Mul(u, u)
  LET u3v AS List OF Integer = __crypto_gf448Mul(__crypto_gf448Mul(u2, u), v)
  LET u5v3 AS List OF Integer = __crypto_gf448Mul(__crypto_gf448Mul(u3v, u2), __crypto_gf448Mul(v, v))
  MUT x AS List OF Integer = __crypto_gf448Mul(u3v, __crypto_gf448PowP34(u5v3))
  LET check AS List OF Integer = __crypto_gf448Mul(v, __crypto_gf448Mul(x, x))
  IF __crypto_constantTimeEqual(__crypto_gf448Pack(check), __crypto_gf448Pack(u)) = FALSE THEN
    RETURN __crypto_zeroLimbs(49)
  END IF
  LET xb AS List OF Byte = __crypto_gf448Pack(x)
  LET xZero AS Boolean = __crypto_isAllZero(xb)
  IF xZero AND x0 = 1 THEN
    RETURN __crypto_zeroLimbs(49)
  END IF
  IF bits::band(toInt(collections::get(xb, 0)), 1) <> x0 THEN
    x = __crypto_gf448Sub(zero, x)
  END IF
  IF xZero OR __crypto_isAllZero(__crypto_gf448Pack(y)) THEN
    RETURN __crypto_zeroLimbs(49)
  END IF
  MUT ok AS List OF Integer = []
  ok = collections::append(ok, 1)
  RETURN __crypto_concatInt(ok, __crypto_ed448Point3(x, y, one))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Decode", BODY));
}
