//! `__crypto_argon2H0` — shared private helper for the `crypto` package.
//!
//! Argon2's pre-hashing digest H_0 (RFC 9106 figure 1).
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Argon2's pre-hashing digest H_0 (RFC 9106 figure 1). The secret and associated-data
' fields this member does not expose are still length-prefixed, as zero-length.
FUNC __crypto_argon2H0(password AS List OF Byte, salt AS List OF Byte, memoryKiB AS Integer, iterations AS Integer, parallelism AS Integer, length AS Integer) AS List OF Byte
  MUT b AS List OF Byte = []
  b = __crypto_argon2Le32(b, parallelism)
  b = __crypto_argon2Le32(b, length)
  b = __crypto_argon2Le32(b, memoryKiB)
  b = __crypto_argon2Le32(b, iterations)
  b = __crypto_argon2Le32(b, 19)
  b = __crypto_argon2Le32(b, 2)
  b = __crypto_argon2Le32(b, len(password))
  b = __crypto_concat(b, password)
  b = __crypto_argon2Le32(b, len(salt))
  b = __crypto_concat(b, salt)
  b = __crypto_argon2Le32(b, 0)
  b = __crypto_argon2Le32(b, 0)
  RETURN __crypto_blake2b(b, 64)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2H0",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
