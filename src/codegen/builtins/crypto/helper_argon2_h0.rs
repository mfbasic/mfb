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
' fields this member does not expose are still length-prefixed, as zero-length. The
' password is hashed where it lies (`__crypto_blake2b3`), never copied (plan-142-E).
FUNC __crypto_argon2H0(password AS List OF Byte, salt AS List OF Byte, memoryKiB AS Integer, iterations AS Integer, parallelism AS Integer, length AS Integer) AS List OF Byte
  MUT pre AS List OF Byte = []
  pre = __crypto_argon2Le32(pre, parallelism)
  pre = __crypto_argon2Le32(pre, length)
  pre = __crypto_argon2Le32(pre, memoryKiB)
  pre = __crypto_argon2Le32(pre, iterations)
  pre = __crypto_argon2Le32(pre, 19)
  pre = __crypto_argon2Le32(pre, 2)
  pre = __crypto_argon2Le32(pre, len(password))
  MUT post AS List OF Byte = []
  post = __crypto_argon2Le32(post, len(salt))
  post = __crypto_concat(post, salt)
  post = __crypto_argon2Le32(post, 0)
  post = __crypto_argon2Le32(post, 0)
  RETURN __crypto_blake2b3(pre, password, post, 64)
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
