//! `__crypto_chacha20Poly1305Seal` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_chacha20Poly1305Seal(key AS List OF Byte, nonce AS List OF Byte, plaintext AS List OF Byte, aad AS List OF Byte) AS Sealed
  IF len(key) <> 32 THEN
    FAIL error(77050002, "chacha20poly1305 key must be 32 bytes")
  END IF
  IF len(nonce) <> 12 THEN
    FAIL error(77050002, "chacha20poly1305 nonce must be 12 bytes")
  END IF
  LET polyKeyFull AS List OF Byte = __crypto_chachaBlock(key, nonce, 0)
  LET polyKey AS List OF Byte = __crypto_truncate(polyKeyFull, 32)
  LET ciphertext AS List OF Byte = __crypto_chacha20(key, nonce, 1, plaintext)
  LET macData AS List OF Byte = __crypto_aeadMacData(aad, ciphertext)
  LET tag AS List OF Byte = __crypto_poly1305(polyKey, macData)
  RETURN Sealed[ciphertext, tag]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_chacha20Poly1305Seal", BODY));
}
