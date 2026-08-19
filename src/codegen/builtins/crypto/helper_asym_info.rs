//! `__crypto_asymInfo` — shared private helper for the `crypto` package.
//!
//! The HKDF `info` string for the X25519 sealed-box construction:
//! `"mfb-box-v1"` ‖ the one-byte `AsymmetricCipher` ordinal (0 for
//! `Ed25519_AES256GCM`, 1 for `Ed25519_CHACHA20POLY1305`). Domain-separates the
//! derived key/nonce per suite. Shared by `__crypto_encrypt`/`__crypto_decrypt` so
//! both derive the identical `okm`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' HKDF info: "mfb-box-v1" followed by the one-byte suite ordinal.
FUNC __crypto_asymInfo(cipher AS AsymmetricCipher) AS List OF Byte
  MUT info AS List OF Byte = strings::toBytes("mfb-box-v1")
  MUT ord AS Integer = 1
  IF cipher = AsymmetricCipher.Ed25519_AES256GCM THEN
    ord = 0
  END IF
  info = collections::append(info, toByte(ord))
  RETURN info
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_asymInfo", BODY));
}
