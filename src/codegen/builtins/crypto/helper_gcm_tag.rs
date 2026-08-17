//! `__crypto_gcmTag` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The 16-byte GCM tag: GHASH(aad,ct) XOR AES(J0).
FUNC __crypto_gcmTag(roundKeys AS List OF Byte, hHi AS Integer, hLo AS Integer, j0 AS List OF Byte, aad AS List OF Byte, ciphertext AS List OF Byte) AS List OF Byte
  LET ghashData AS List OF Byte = __crypto_gcmGhashData(aad, ciphertext)
  LET s AS List OF Integer = __crypto_ghash(hHi, hLo, ghashData)
  MUT block AS List OF Byte = []
  block = __crypto_appendBeWord64(block, collections::get(s, 0))
  block = __crypto_appendBeWord64(block, collections::get(s, 1))
  LET ej0 AS List OF Byte = __crypto_aesEncryptBlock(roundKeys, j0)
  RETURN __crypto_xorBytes(block, ej0)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gcmTag", BODY));
}
