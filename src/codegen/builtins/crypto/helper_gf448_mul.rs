//! `__crypto_gf448Mul` — shared private helper for the `crypto` package.
//!
//! Schoolbook product of two carried 16 × 28-bit limb vectors, reduced with the
//! Goldilocks identity `2^448 ≡ 2^224 + 1 (mod p)`: convolution column `k ≥ 16`
//! folds into limbs `k−16` and `k−8`, and a column `k ≥ 24` folds a second time
//! (limb `k−16` twice, limb `k−24` once). Every accumulator is a sum of at most
//! 38 products of two limbs `≤ 2^28`, i.e. `< 2^61.3` — inside the trapping
//! `Integer` (the exact maxima are pinned by `gf448_mul_accumulators_fit_i63`).
//! Every branch is on a loop counter; nothing depends on the limb values.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GF(2^448-2^224-1) multiplication: 31 convolution columns folded by 2^448 = 2^224 + 1.
FUNC __crypto_gf448Mul(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  MUT c AS List OF Integer = []
  MUT k AS Integer = 0
  WHILE k < 31
    MUT acc AS Integer = 0
    MUT i AS Integer = 0
    WHILE i < 16
      LET j AS Integer = k - i
      IF j >= 0 AND j <= 15 THEN
        acc = acc + collections::get(a, i) * collections::get(b, j)
      END IF
      i = i + 1
    END WHILE
    c = collections::append(c, acc)
    k = k + 1
  END WHILE
  MUT o AS List OF Integer = []
  MUT n AS Integer = 0
  WHILE n < 16
    MUT v AS Integer = collections::get(c, n)
    IF n <= 14 THEN
      v = v + collections::get(c, n + 16)
    END IF
    IF n >= 8 THEN
      v = v + collections::get(c, n + 8)
      IF n <= 14 THEN
        v = v + collections::get(c, n + 16)
      END IF
    END IF
    IF n <= 6 THEN
      v = v + collections::get(c, n + 24)
    END IF
    o = collections::append(o, v)
    n = n + 1
  END WHILE
  RETURN __crypto_gf448Carry(__crypto_gf448Carry(o))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448Mul", BODY));
}
