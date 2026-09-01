//! `__crypto_pbkdf2` — shared private helper for the `crypto` package.
//!
//! The hash-generic PBKDF2 (RFC 2898) key-derivation ladder, written over the
//! hash-generic `__crypto_hmac`. It reuses the already hash-agnostic
//! `__crypto_pbkdf2Block` per-block function, supplying it a hash-generic HMAC closure
//! that binds the `Hash` selector — so the same construction serves every `Hash`
//! variant. It is the single MFB body behind the unified `crypto::pbkdf2(Hash, …)`
//! member.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Hash-generic PBKDF2 (RFC 2898) over the hash-generic HMAC of the selected `crypto::Hash`.
FUNC __crypto_pbkdf2(algo AS Hash, password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte
  IF iterations < 1 OR length < 1 THEN
    FAIL error(77050002, "pbkdf2 iterations/length out of range")
  END IF
  MUT okm AS List OF Byte = []
  MUT index AS Integer = 1
  WHILE len(okm) < length
    okm = __crypto_concat(okm, __crypto_pbkdf2Block(password, salt, iterations, index, LAMBDA(mk AS List OF Byte, md AS List OF Byte) -> __crypto_hmac(algo, mk, md)))
    index = index + 1
  END WHILE
  RETURN __crypto_truncate(okm, length)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pbkdf2", BODY));
}
