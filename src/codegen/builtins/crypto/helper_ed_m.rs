//! `__crypto_edM` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_edM(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  MUT o AS List OF Integer = []
  MUT k AS Integer = 0
  WHILE k < 16
    MUT acc AS Integer = 0
    MUT i AS Integer = 0
    WHILE i < 16
      LET j AS Integer = k - i
      IF j >= 0 AND j <= 15 THEN
        acc = acc + collections::get(a, i) * collections::get(b, j)
      END IF
      LET j2 AS Integer = k + 16 - i
      IF j2 >= 0 AND j2 <= 15 THEN
        acc = acc + 38 * (collections::get(a, i) * collections::get(b, j2))
      END IF
      i = i + 1
    END WHILE
    o = collections::append(o, acc)
    k = k + 1
  END WHILE
  o = __crypto_car25519(o)
  o = __crypto_car25519(o)
  RETURN o
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_edM", BODY));
}
