//! `__crypto_encryptText` — shared private helper for the `crypto` package.
//!
//! The `String` `data` overload of `crypto::encrypt(cipher, recipientPublicKey,
//! data[, aad])` rewrites to this shim: it UTF-8-encodes `data` and re-enters the
//! `List OF Byte` `encrypt`, so a `String` argument reaches the same RFC 9180 HPKE
//! construction as raw bytes (identical to the `hash`/`seal` `_text` shims).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_encryptText(cipher AS AsymmetricCipher, recipientPublicKey AS List OF Byte, data AS String, aad AS List OF Byte) AS List OF Byte
  RETURN crypto::encrypt(cipher, recipientPublicKey, strings::toBytes(data), aad)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_encryptText", BODY));
}
