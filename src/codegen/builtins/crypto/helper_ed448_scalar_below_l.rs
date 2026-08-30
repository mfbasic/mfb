//! `__crypto_ed448ScalarBelowL` — shared private helper for the `crypto` package.
//!
//! TRUE iff the 57-byte little-endian scalar `s` is canonical: its top byte is
//! zero and its value is strictly below the group order `L`. An Ed448 signature
//! whose `S` is `≥ L` is non-canonical (malleable), so `__crypto_ed448Verify`
//! rejects it before doing any curve work — the bug-269 / CRY-02 rule the
//! Ed25519 verifier already applies. `S` is public, so the early exit is fine.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' TRUE iff the 57-byte little-endian scalar is canonical (top byte 0, value < L).
FUNC __crypto_ed448ScalarBelowL(s AS List OF Byte) AS Boolean
  IF toInt(collections::get(s, 56)) <> 0 THEN
    RETURN FALSE
  END IF
  MUT i AS Integer = 55
  WHILE i >= 0
    LET si AS Integer = toInt(collections::get(s, i))
    LET li AS Integer = collections::get(__CRYPTO_ED448_L, i)
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
    pkg.add_helper(RegistryHelper::always("crypto_ed448ScalarBelowL", BODY));
}
