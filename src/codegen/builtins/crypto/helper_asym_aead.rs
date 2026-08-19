//! `__crypto_asymAead` — shared private helper for the `crypto` package.
//!
//! Maps a `crypto::AsymmetricCipher` suite to the underlying symmetric AEAD
//! `crypto::SymmetricCipher` used by its sealed-box construction:
//! `Ed25519_AES256GCM` → `AES256GCM`, `Ed25519_CHACHA20POLY1305` →
//! `CHACHA20POLY1305`. Shared by `__crypto_encrypt`/`__crypto_decrypt` so both halves
//! agree on the AEAD.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Underlying symmetric AEAD for an AsymmetricCipher suite.
FUNC __crypto_asymAead(cipher AS AsymmetricCipher) AS SymmetricCipher
  MUT aead AS SymmetricCipher = SymmetricCipher.CHACHA20POLY1305
  IF cipher = AsymmetricCipher.Ed25519_AES256GCM THEN
    aead = SymmetricCipher.AES256GCM
  END IF
  RETURN aead
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_asymAead", BODY));
}
