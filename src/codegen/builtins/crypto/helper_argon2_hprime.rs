//! `__crypto_argon2HPrime` — shared private helper for the `crypto` package.
//!
//! Argon2's variable-length hash H' (RFC 9106 §3.3), which is how a 64-byte BLAKE2b
//! reaches the 1024-byte starting blocks and any requested tag length.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Argon2's variable-length hash H' (RFC 9106 section 3.3).
FUNC __crypto_argon2HPrime(a AS List OF Byte, outLen AS Integer) AS List OF Byte
  MUT inp AS List OF Byte = []
  inp = __crypto_argon2Le32(inp, outLen)
  inp = __crypto_concat(inp, a)
  IF outLen <= 64 THEN
    RETURN __crypto_blake2b(inp, outLen)
  END IF
  LET r AS Integer = (outLen + 31) / 32 - 2
  MUT v AS List OF Byte = __crypto_blake2b(inp, 64)
  MUT out AS List OF Byte = __crypto_truncate(v, 32)
  MUT i AS Integer = 1
  WHILE i < r
    v = __crypto_blake2b(v, 64)
    out = __crypto_concat(out, __crypto_truncate(v, 32))
    i = i + 1
  END WHILE
  RETURN __crypto_concat(out, __crypto_blake2b(v, outLen - 32 * r))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2HPrime",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
