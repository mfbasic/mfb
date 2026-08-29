//! `__crypto_ed448Scalarmult` — shared private helper for the `crypto` package.
//!
//! `[s]Q` for a 57-byte little-endian scalar over edwards448: a Montgomery-style
//! ladder over the packed `(P, Q)` pair with 448 fixed iterations (bit 447 down
//! to 0 — every Ed448 scalar is `< 2^448`, RFC 8032's pruning sets bit 447 and
//! zeroes byte 56), one branch-free `__crypto_ed448Cswap` before and after each
//! unified-addition step. No control flow depends on the scalar or the point;
//! the same shape as `__crypto_scalarmult` for Ed25519.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' [s]Q over edwards448: a 448-step select-swap ladder over the projective pair.
FUNC __crypto_ed448Scalarmult(q AS List OF Integer, s AS List OF Byte) AS List OF Integer
  MUT pair AS List OF Integer = __crypto_concatInt(__crypto_ed448Identity(), q)
  MUT i AS Integer = 447
  WHILE i >= 0
    LET b AS Integer = bits::band(bits::sr(toInt(collections::get(s, bits::sr(i, 3))), bits::band(i, 7)), 1)
    pair = __crypto_ed448Cswap(pair, b)
    LET pp AS List OF Integer = collections::mid(pair, 0, 48)
    LET qq AS List OF Integer = collections::mid(pair, 48, 48)
    LET newQ AS List OF Integer = __crypto_ed448Add(qq, pp)
    LET newP AS List OF Integer = __crypto_ed448Add(pp, pp)
    pair = __crypto_concatInt(newP, newQ)
    pair = __crypto_ed448Cswap(pair, b)
    i = i - 1
  END WHILE
  RETURN collections::mid(pair, 0, 48)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Scalarmult", BODY));
}
