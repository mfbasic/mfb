//! `__crypto_scalarBelowL` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-269 / CRY-02: TRUE iff the 32-byte little-endian scalar `s` is strictly less
' than the group order L. Ed25519 signatures whose S component is >= L are
' non-canonical and malleable — a third party can transform a valid signature into
' a distinct still-verifying one over the same message without the key. Rejecting
' S >= L keeps the signature bytes a stable identity (safe as a dedup/replay key).
FUNC __crypto_scalarBelowL(s AS List OF Byte) AS Boolean
  LET order AS List OF Integer = __crypto_edL()
  MUT i AS Integer = 31
  WHILE i >= 0
    LET si AS Integer = toInt(collections::get(s, i))
    LET li AS Integer = collections::get(order, i)
    IF si < li THEN
      RETURN TRUE
    END IF
    IF si > li THEN
      RETURN FALSE
    END IF
    i = i - 1
  END WHILE
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_scalarBelowL", BODY));
}
