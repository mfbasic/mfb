//! `__crypto_hkdfSha256` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' HKDF-Extract + Expand over HMAC-SHA-256 (hashLen 32).
FUNC __crypto_hkdfSha256(ikm AS List OF Byte, salt AS List OF Byte, info AS List OF Byte, length AS Integer) AS List OF Byte
  IF length < 1 OR length > 8160 THEN
    FAIL error(77050002, "hkdf length out of range")
  END IF
  MUT usedSalt AS List OF Byte = salt
  IF len(usedSalt) = 0 THEN
    usedSalt = __crypto_zeroPad([], 32)
  END IF
  LET prk AS List OF Byte = __crypto_hmacSha256_bytes(usedSalt, ikm)
  RETURN __crypto_hkdfExpand(prk, info, length, __crypto_hmacSha256_bytes)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hkdfSha256", BODY));
}
