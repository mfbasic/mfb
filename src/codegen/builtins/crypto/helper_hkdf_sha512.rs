//! `__crypto_hkdfSha512` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' HKDF over HMAC-SHA-512 (hashLen 64).
FUNC __crypto_hkdfSha512(ikm AS List OF Byte, salt AS List OF Byte, info AS List OF Byte, length AS Integer) AS List OF Byte
  IF length < 1 OR length > 16320 THEN
    FAIL error(77050002, "hkdf length out of range")
  END IF
  MUT usedSalt AS List OF Byte = salt
  IF len(usedSalt) = 0 THEN
    usedSalt = __crypto_zeroPad([], 64)
  END IF
  LET prk AS List OF Byte = __crypto_hmacSha512_bytes(usedSalt, ikm)
  RETURN __crypto_hkdfExpand(prk, info, length, __crypto_hmacSha512_bytes)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hkdfSha512", BODY));
}
