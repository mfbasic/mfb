//! `__crypto_ed448Add` — shared private helper for the `crypto` package.
//!
//! Unified projective point addition on edwards448 (RFC 8032 §5.2.4, the
//! `a = 1`, `d = −39081` Edwards addition law): complete for this curve's
//! non-square `d`, so it also serves as doubling (`add(p, p)`) and the ladder
//! never branches on the point. `E = d·C·D` is formed as `−(39081·C·D)`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Unified projective edwards448 addition (RFC 8032 §5.2.4): returns p + q.
FUNC __crypto_ed448Add(p AS List OF Integer, q AS List OF Integer) AS List OF Integer
  LET x1 AS List OF Integer = __crypto_ed448PointAt(p, 0)
  LET y1 AS List OF Integer = __crypto_ed448PointAt(p, 1)
  LET z1 AS List OF Integer = __crypto_ed448PointAt(p, 2)
  LET x2 AS List OF Integer = __crypto_ed448PointAt(q, 0)
  LET y2 AS List OF Integer = __crypto_ed448PointAt(q, 1)
  LET z2 AS List OF Integer = __crypto_ed448PointAt(q, 2)
  LET a AS List OF Integer = __crypto_gf448Mul(z1, z2)
  LET b AS List OF Integer = __crypto_gf448Mul(a, a)
  LET c AS List OF Integer = __crypto_gf448Mul(x1, x2)
  LET d AS List OF Integer = __crypto_gf448Mul(y1, y2)
  LET e AS List OF Integer = __crypto_gf448Sub(__crypto_gf448Zero(), __crypto_gf448MulSmall(__crypto_gf448Mul(c, d), 39081))
  LET f AS List OF Integer = __crypto_gf448Sub(b, e)
  LET g AS List OF Integer = __crypto_gf448Add(b, e)
  LET h AS List OF Integer = __crypto_gf448Mul(__crypto_gf448Add(x1, y1), __crypto_gf448Add(x2, y2))
  LET x3 AS List OF Integer = __crypto_gf448Mul(__crypto_gf448Mul(a, f), __crypto_gf448Sub(__crypto_gf448Sub(h, c), d))
  LET y3 AS List OF Integer = __crypto_gf448Mul(__crypto_gf448Mul(a, g), __crypto_gf448Sub(d, c))
  LET z3 AS List OF Integer = __crypto_gf448Mul(f, g)
  RETURN __crypto_ed448Point3(x3, y3, z3)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Add", BODY));
}
