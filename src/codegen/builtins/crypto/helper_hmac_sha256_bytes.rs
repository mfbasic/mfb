//! `__crypto_hmacSha256_bytes` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' HMAC-SHA-256 over a `List OF Byte` message. Block size 64, digest 32.
FUNC __crypto_hmacSha256_bytes(key AS List OF Byte, data AS List OF Byte) AS List OF Byte
  MUT k AS List OF Byte = key
  IF len(k) > 64 THEN
    k = __crypto_sha256_bytes(k)
  END IF
  k = __crypto_zeroPad(k, 64)
  LET inner AS List OF Byte = __crypto_xorPad(k, 54)
  LET outer AS List OF Byte = __crypto_xorPad(k, 92)
  LET innerHash AS List OF Byte = __crypto_sha256_bytes(__crypto_concat(inner, data))
  RETURN __crypto_sha256_bytes(__crypto_concat(outer, innerHash))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hmacSha256_bytes", BODY));
}
