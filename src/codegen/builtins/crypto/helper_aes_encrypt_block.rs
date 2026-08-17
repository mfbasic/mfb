//! `__crypto_aesEncryptBlock` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' AES-256 encrypt the 16-byte `block` under the expanded `roundKeys`.
FUNC __crypto_aesEncryptBlock(roundKeys AS List OF Byte, block AS List OF Byte) AS List OF Byte
  MUT s AS List OF Byte = __crypto_copyBytes(block)
  s = __crypto_aesAddRoundKey(s, roundKeys, 0)
  MUT round AS Integer = 1
  WHILE round < 14
    s = __crypto_aesSubBytes(s)
    s = __crypto_aesShiftRows(s)
    s = __crypto_aesMixColumns(s)
    s = __crypto_aesAddRoundKey(s, roundKeys, round)
    round = round + 1
  END WHILE
  s = __crypto_aesSubBytes(s)
  s = __crypto_aesShiftRows(s)
  s = __crypto_aesAddRoundKey(s, roundKeys, 14)
  RETURN s
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aesEncryptBlock", BODY));
}
