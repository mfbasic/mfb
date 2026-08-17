//! `__crypto_scalarmult` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Montgomery-style ladder over the extended-coordinate point `q`, constant-time
' via a masked conditional swap of the packed (p||q) pair.
' Performance: one scalar multiplication is ~15000 small `List OF Integer`
' allocations. This was impractically slow under the original arena allocator, but
' the allocator work has since landed (plan-25-A and successors) and a full
' keygen + sign + verify now completes in well under a second (bug-339 D5
' re-benchmark, 2026-07-25). The old "blocked on the bug-01 allocator fix" note
' referenced a document that no longer exists in the tree; removed.
FUNC __crypto_scalarmult(q AS List OF Integer, s AS List OF Byte) AS List OF Integer
  LET p AS List OF Integer = __crypto_point4(__crypto_gf0(), __crypto_gf1(), __crypto_gf1(), __crypto_gf0())
  MUT pair AS List OF Integer = __crypto_concatInt(p, q)
  MUT i AS Integer = 255
  WHILE i >= 0
    LET byteIdx AS Integer = i / 8
    LET bitIdx AS Integer = bits::band(i, 7)
    LET b AS Integer = bits::band(bits::sr(toInt(collections::get(s, byteIdx)), bitIdx), 1)
    pair = __crypto_cswap128(pair, b)
    LET pp AS List OF Integer = __crypto_first64(pair)
    LET qq AS List OF Integer = __crypto_last64(pair)
    LET newQ AS List OF Integer = __crypto_edAdd(qq, pp)
    LET newP AS List OF Integer = __crypto_edAdd(pp, pp)
    pair = __crypto_concatInt(newP, newQ)
    pair = __crypto_cswap128(pair, b)
    i = i - 1
  END WHILE
  RETURN __crypto_first64(pair)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_scalarmult", BODY));
}
