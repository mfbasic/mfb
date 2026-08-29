//! `__crypto_ed448Fold` — shared private helper for the `crypto` package.
//!
//! One reduction step for the Ed448 scalar field: with `x = hi·2^446 + lo`
//! (`lo` the low 446 bits — 55 whole bytes plus 6 bits of byte 55; `hi` the rest,
//! shifted down by 6), return `hi·c + lo`, since `2^446 ≡ c (mod L)`. Each fold
//! removes ~224 bits; `__crypto_ed448ModL` applies three (114 → 89 → 64 → 57
//! bytes, the last below `2^446 + 2^272 < 2L`) and one masked subtraction. The
//! input must have at least 57 limbs (`__crypto_padLimbs`).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' One 2^446 = c (mod L) fold of a byte-limb integer (>= 57 limbs): hi * c + lo.
FUNC __crypto_ed448Fold(x AS List OF Integer) AS List OF Integer
  MUT lo AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 55
    lo = collections::append(lo, collections::get(x, i))
    i = i + 1
  END WHILE
  lo = collections::append(lo, bits::band(collections::get(x, 55), 63))
  MUT hi AS List OF Integer = []
  LET n AS Integer = len(x) - 56
  i = 0
  WHILE i < n
    LET h AS Integer = bits::bor(bits::sr(collections::get(x, 55 + i), 6), bits::sl(bits::band(collections::get(x, 56 + i), 63), 2))
    hi = collections::append(hi, h)
    i = i + 1
  END WHILE
  hi = collections::append(hi, bits::sr(collections::get(x, len(x) - 1), 6))
  RETURN __crypto_bnAdd(__crypto_bnMul(hi, __CRYPTO_ED448_C), lo)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Fold", BODY));
}
