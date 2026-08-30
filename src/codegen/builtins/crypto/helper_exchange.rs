//! `__crypto_exchange` — shared private helper for the `crypto` package.
//!
//! The pure-MFB core behind `crypto::exchange(type, privateKey, publicKey)`:
//! Diffie-Hellman over the key-agreement `Certificate`s — `X25519`
//! (`__crypto_x25519`, 32-byte keys) and `X448` (`__crypto_x448`, 56-byte keys).
//! A signing certificate, a wrong-length key, or an all-zero shared secret (a
//! low-order peer point, RFC 7748 §6.1) fails with `ErrInvalidArgument`; the
//! caller never sees a degenerate secret.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' X25519 / X448 Diffie-Hellman; fails closed on a non-key-agreement type, a
' wrong key length, or an all-zero shared secret (RFC 7748 §6.1).
FUNC __crypto_exchange(cert AS Certificate, privateKey AS List OF Byte, publicKey AS List OF Byte) AS List OF Byte
  MUT shared AS List OF Byte = []
  IF cert = Certificate.X25519 THEN
    IF len(privateKey) <> 32 OR len(publicKey) <> 32 THEN
      FAIL error(77050002, "X25519 keys must be 32 bytes")
    END IF
    shared = __crypto_x25519(privateKey, publicKey)
  ELSE
    IF cert = Certificate.X448 THEN
      IF len(privateKey) <> 56 OR len(publicKey) <> 56 THEN
        FAIL error(77050002, "X448 keys must be 56 bytes")
      END IF
      shared = __crypto_x448(privateKey, publicKey)
    ELSE
      FAIL error(77050002, "exchange requires a key-agreement certificate (X25519 or X448)")
    END IF
  END IF
  IF __crypto_isAllZero(shared) THEN
    FAIL error(77050002, "exchange produced an all-zero shared secret (low-order public key)")
  END IF
  RETURN shared
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_exchange", BODY));
}
