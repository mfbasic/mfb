//! `__crypto_aes256GcmOpen` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_aes256GcmOpen(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte, aad AS List OF Byte) AS List OF Byte
  IF len(key) <> 32 THEN
    FAIL error(77050002, "aes256gcm key must be 32 bytes")
  END IF
  IF len(nonce) <> 12 THEN
    FAIL error(77050002, "aes256gcm nonce must be 12 bytes")
  END IF
  LET roundKeys AS List OF Byte = __crypto_aesExpandKey(key)
  LET zeros AS List OF Byte = __crypto_zeroPad([], 16)
  LET hBlock AS List OF Byte = __crypto_aesEncryptBlock(roundKeys, zeros)
  LET hHi AS Integer = __crypto_beWord64(hBlock, 0)
  LET hLo AS Integer = __crypto_beWord64(hBlock, 8)
  LET j0 AS List OF Byte = __crypto_gcmJ0(nonce)
  LET expected AS List OF Byte = __crypto_gcmTag(roundKeys, hHi, hLo, j0, aad, ciphertext)
  IF __crypto_constantTimeEqual(expected, tag) = FALSE THEN
    FAIL error(77050016, "aes256gcm authentication failed")
  END IF
  LET ctr AS List OF Byte = __crypto_gcmInc32(j0)
  RETURN __crypto_gcmGctr(roundKeys, ctr, ciphertext)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aes256GcmOpen", BODY));
}
