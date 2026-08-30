//! `__crypto_gf448Carry` — shared private helper for the `crypto` package.
//!
//! One carry pass over the 16 × 28-bit limbs: every limb is masked to 28 bits
//! and its overflow rides into the next; the overflow out of limb 15 (a multiple
//! of 2^448 ≡ 2^224 + 1 mod p) folds back into limbs 0 and 8. Two passes bring
//! any non-negative limb vector below 2^63 back to limbs in `0..=2^28` (a third
//! guarantees strictly `< 2^28`, which `__crypto_gf448Pack` needs). Limbs must
//! be non-negative on entry (`__crypto_gf448Sub` biases by 2p to ensure it), so
//! the logical `bits::sr` is the carry.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' One GF(2^448-2^224-1) carry pass; the top carry folds into limbs 0 and 8.
FUNC __crypto_gf448Carry(o AS List OF Integer) AS List OF Integer
  MUT r AS List OF Integer = o
  MUT carry AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 16
    LET v AS Integer = collections::get(r, i) + carry
    r = collections::set(r, i, bits::band(v, 268435455))
    carry = bits::sr(v, 28)
    i = i + 1
  END WHILE
  r = collections::set(r, 0, collections::get(r, 0) + carry)
  r = collections::set(r, 8, collections::get(r, 8) + carry)
  RETURN r
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448Carry", BODY));
}
