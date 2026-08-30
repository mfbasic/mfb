//! `__crypto_gf448PowP34` — shared private helper for the `crypto` package.
//!
//! `a^((p−3)/4)` — the exponent of the RFC 8032 §5.2.3 square-root recovery
//! (`x = u³·v·(u⁵·v³)^((p−3)/4)`) for `p ≡ 3 (mod 4)`. `(p − 3)/4 = 2^446 − 2^222 − 1`
//! is 446 bits whose only zero bit is 222, so this is a fixed left-to-right
//! square-and-multiply (bit 445 seeds `c = a`; multiply at every lower bit but
//! 222).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' a^((p-3)/4) over GF(2^448-2^224-1); (p-3)/4 = 2^446 - 2^222 - 1 (only bit 222 clear).
FUNC __crypto_gf448PowP34(a AS List OF Integer) AS List OF Integer
  MUT c AS List OF Integer = a
  MUT i AS Integer = 444
  WHILE i >= 0
    c = __crypto_gf448Mul(c, c)
    IF i <> 222 THEN
      c = __crypto_gf448Mul(c, a)
    END IF
    i = i - 1
  END WHILE
  RETURN c
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gf448PowP34", BODY));
}
