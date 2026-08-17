//! `__crypto_pbkdf2Sha512_bytes` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_pbkdf2Sha512_bytes(password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte
  IF iterations < 1 OR length < 1 THEN
    FAIL error(77050002, "pbkdf2 iterations/length out of range")
  END IF
  MUT okm AS List OF Byte = []
  MUT index AS Integer = 1
  WHILE len(okm) < length
    okm = __crypto_concat(okm, __crypto_pbkdf2Block(password, salt, iterations, index, __crypto_hmacSha512_bytes))
    index = index + 1
  END WHILE
  RETURN __crypto_truncate(okm, length)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pbkdf2Sha512_bytes", BODY));
}
