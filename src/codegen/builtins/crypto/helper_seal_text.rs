//! `__crypto_sealText` — shared private helper for the `crypto` package.
//!
//! The `String` `data` overload of `crypto::seal(cipher, key, nonce, data[, aad])`
//! rewrites to this shim: it UTF-8-encodes `data` and re-enters the `List OF Byte` `seal`
//! `AbiFunction`, so a `String` argument reaches the same per-ordinal AEAD dispatch as raw
//! bytes (identical to the `hash` `_text` shim). It cannot be a second `AbiFunction`
//! overload: an `AbiFunction` member emits a single `crypto.seal` runtime helper whose
//! body is the first (`List OF Byte`) overload, and a `String` pointer read through that
//! `List OF Byte`-shaped body faults.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled source
//! (before the member bodies), in the order `mod.rs` calls the helpers. Body
//! byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sealText(cipher AS SymmetricCipher, key AS List OF Byte, nonce AS List OF Byte, data AS String, aad AS List OF Byte) AS Sealed
  RETURN crypto::seal(cipher, key, nonce, strings::toBytes(data), aad)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sealText", BODY));
}
