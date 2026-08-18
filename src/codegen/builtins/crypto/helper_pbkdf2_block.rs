//! `__crypto_pbkdf2Block` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C6: one PBKDF2 block function parameterized by the HMAC primitive,
' instead of byte-identical __crypto_pbkdf2Block256/512 differing only in the hash.
FUNC __crypto_pbkdf2Block(password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, index AS Integer, hmac AS FUNC(List OF Byte, List OF Byte) AS List OF Byte) AS List OF Byte
  LET first AS List OF Byte = __crypto_concat(salt, __crypto_be32(index))
  MUT u AS List OF Byte = hmac(password, first)
  MUT result AS List OF Byte = u
  MUT j AS Integer = 1
  WHILE j < iterations
    u = hmac(password, u)
    result = __crypto_xorBytes(result, u)
    j = j + 1
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pbkdf2Block", BODY));
}
