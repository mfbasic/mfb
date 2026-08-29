//! `__crypto_gf448Sub` — shared private helper for the `crypto` package.
//!
//! `a − b` biased by `2p` limb-wise so no limb goes negative (the carry pass
//! relies on non-negative limbs): `2p` in 28-bit limbs is `2·(2^28−1)` everywhere
//! except limb 8, which is `2·(2^28−2)` (the `−2^224` term of the prime).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GF(2^448-2^224-1) subtraction a - b + 2p (limb-wise bias keeps limbs >= 0).
FUNC __crypto_gf448Sub(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    MUT bias AS Integer = 536870910
    IF i = 8 THEN
      bias = 536870908
    END IF
    o = collections::append(o, collections::get(a, i) - collections::get(b, i) + bias)
    i = i + 1
  END WHILE
  RETURN __crypto_gf448Carry(__crypto_gf448Carry(o))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448Sub", BODY));
}
