//! `__crypto_bnMul` — shared private helper for the `crypto` package.
//!
//! Schoolbook product of two little-endian byte-limb integers (every limb
//! `0..255`), normalised to byte limbs. The carry out of each row is propagated
//! through every remaining position unconditionally, so the work depends only on
//! the (public) operand lengths, never on the limb values. Used by the Ed448
//! scalar arithmetic (`k·s`, `hi·C`): a 114 × 57 product's column sum is below
//! `2^24`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Byte-limb big-integer product (little-endian limbs 0..255), fixed-length carries.
FUNC __crypto_bnMul(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  LET n AS Integer = len(a) + len(b) + 1
  MUT out AS List OF Integer = __crypto_zeroLimbs(n)
  MUT i AS Integer = 0
  WHILE i < len(a)
    MUT carry AS Integer = 0
    MUT j AS Integer = 0
    WHILE j < len(b)
      LET v AS Integer = collections::get(out, i + j) + collections::get(a, i) * collections::get(b, j) + carry
      out = collections::set(out, i + j, bits::band(v, 255))
      carry = bits::sr(v, 8)
      j = j + 1
    END WHILE
    MUT k AS Integer = i + len(b)
    WHILE k < n
      LET w AS Integer = collections::get(out, k) + carry
      out = collections::set(out, k, bits::band(w, 255))
      carry = bits::sr(w, 8)
      k = k + 1
    END WHILE
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_bnMul", BODY));
}
