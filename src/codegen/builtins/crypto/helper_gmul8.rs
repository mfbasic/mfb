//! `__crypto_gmul8` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' GF(2^8) multiply of `a` by `b`.
FUNC __crypto_gmul8(a AS Integer, b AS Integer) AS Integer
  MUT p AS Integer = 0
  MUT x AS Integer = a
  MUT y AS Integer = b
  MUT i AS Integer = 0
  WHILE i < 8
    IF bits::band(y, 1) <> 0 THEN
      p = bits::bxor(p, x)
    END IF
    x = __crypto_xtime(x)
    y = bits::sr(y, 1)
    i = i + 1
  END WHILE
  RETURN bits::band(p, 255)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gmul8", BODY));
}
