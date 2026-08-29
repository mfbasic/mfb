//! `__crypto_gf448Inv` — shared private helper for the `crypto` package.
//!
//! Inversion by Fermat: `a^(p−2)` with `p − 2 = 2^448 − 2^224 − 3 =
//! (2^224 − 2)·2^224 + (2^224 − 3)`, whose binary expansion is all ones except
//! bits 224 and 1. A fixed left-to-right square-and-multiply (bit 447 seeds
//! `c = a`; for every lower bit square, and multiply unless the bit is 224 or 1)
//! — the same fixed-sequence shape as `__crypto_inv25519`. `a = 0` yields 0.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GF(2^448-2^224-1) inverse: a^(p-2), p-2 = 2^448 - 2^224 - 3 (all bits set but 224 and 1).
FUNC __crypto_gf448Inv(a AS List OF Integer) AS List OF Integer
  MUT c AS List OF Integer = a
  MUT i AS Integer = 446
  WHILE i >= 0
    c = __crypto_gf448Mul(c, c)
    IF i <> 1 AND i <> 224 THEN
      c = __crypto_gf448Mul(c, a)
    END IF
    i = i - 1
  END WHILE
  RETURN c
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448Inv", BODY));
}
