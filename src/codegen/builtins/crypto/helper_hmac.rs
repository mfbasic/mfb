//! `__crypto_hmac` — shared private helper for the `crypto` package.
//!
//! The hash-generic HMAC core (RFC 2104), written over the `__crypto_shaDigest` /
//! `__crypto_shaBlockSize` dispatch instead of a hardcoded SHA. It is the single MFB
//! body behind the unified `crypto::hmac(Hash, key, data)` member (the `List OF Byte`
//! overload rewrites to it) and the KDF ladders below key their HMAC through it, so every
//! `Hash` variant — present and future — is authenticated by this one construction.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Hash-generic HMAC over a `List OF Byte` message. Block size B and digest from the
' `crypto::Hash` selector (RFC 2104). ipad=0x36=54, opad=0x5c=92.
FUNC __crypto_hmac(algo AS Hash, key AS List OF Byte, data AS List OF Byte) AS List OF Byte
  LET b AS Integer = __crypto_shaBlockSize(algo)
  MUT k AS List OF Byte = key
  IF len(k) > b THEN
    k = __crypto_shaDigest(algo, k)
  END IF
  k = __crypto_zeroPad(k, b)
  LET inner AS List OF Byte = __crypto_xorPad(k, 54)
  LET outer AS List OF Byte = __crypto_xorPad(k, 92)
  LET innerHash AS List OF Byte = __crypto_shaDigest(algo, __crypto_concat(inner, data))
  RETURN __crypto_shaDigest(algo, __crypto_concat(outer, innerHash))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hmac", BODY));
}
