//! `__crypto_edAdd` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Edwards point addition (extended coordinates): returns p + q.
FUNC __crypto_edAdd(p AS List OF Integer, q AS List OF Integer) AS List OF Integer
  LET px AS List OF Integer = __crypto_gfAt(p, 0)
  LET py AS List OF Integer = __crypto_gfAt(p, 1)
  LET pz AS List OF Integer = __crypto_gfAt(p, 2)
  LET pt AS List OF Integer = __crypto_gfAt(p, 3)
  LET qx AS List OF Integer = __crypto_gfAt(q, 0)
  LET qy AS List OF Integer = __crypto_gfAt(q, 1)
  LET qz AS List OF Integer = __crypto_gfAt(q, 2)
  LET qt AS List OF Integer = __crypto_gfAt(q, 3)
  MUT a AS List OF Integer = __crypto_edZ(py, px)
  LET t1 AS List OF Integer = __crypto_edZ(qy, qx)
  a = __crypto_edM(a, t1)
  MUT b AS List OF Integer = __crypto_edA(px, py)
  LET t2 AS List OF Integer = __crypto_edA(qx, qy)
  b = __crypto_edM(b, t2)
  MUT c AS List OF Integer = __crypto_edM(pt, qt)
  c = __crypto_edM(c, __crypto_gfD2())
  MUT d AS List OF Integer = __crypto_edM(pz, qz)
  d = __crypto_edA(d, d)
  LET e AS List OF Integer = __crypto_edZ(b, a)
  LET f AS List OF Integer = __crypto_edZ(d, c)
  LET g AS List OF Integer = __crypto_edA(d, c)
  LET h AS List OF Integer = __crypto_edA(b, a)
  LET x3 AS List OF Integer = __crypto_edM(e, f)
  LET y3 AS List OF Integer = __crypto_edM(h, g)
  LET z3 AS List OF Integer = __crypto_edM(g, f)
  LET t3 AS List OF Integer = __crypto_edM(e, h)
  RETURN __crypto_point4(x3, y3, z3, t3)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_edAdd", BODY));
}
