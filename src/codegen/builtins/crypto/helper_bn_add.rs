//! `__crypto_bnAdd` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Byte-limb big-integer sum; the result has max(len(a), len(b)) + 1 limbs.
FUNC __crypto_bnAdd(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  MUT n AS Integer = len(a)
  IF len(b) > n THEN
    n = len(b)
  END IF
  LET pa AS List OF Integer = __crypto_padLimbs(a, n + 1)
  LET pb AS List OF Integer = __crypto_padLimbs(b, n + 1)
  MUT out AS List OF Integer = []
  MUT carry AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < n + 1
    LET v AS Integer = collections::get(pa, i) + collections::get(pb, i) + carry
    out = collections::append(out, bits::band(v, 255))
    carry = bits::sr(v, 8)
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_bnAdd", BODY));
}
