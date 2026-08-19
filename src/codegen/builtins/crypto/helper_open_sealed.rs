//! `__crypto_openSealed` — shared private helper for the `crypto` package.
//!
//! The `crypto::Sealed` overload of `crypto::open(cipher, key, nonce, sealed[, aad])`
//! rewrites to this shim: it unpacks the record's `ciphertext`/`tag` fields and re-enters
//! the five-argument `List OF Byte` `open` `AbiFunction`, so passing a `Sealed` record
//! reaches the same per-ordinal AEAD dispatch as explicit `ciphertext`/`tag` arguments. It
//! cannot be a second `AbiFunction` overload: an `AbiFunction` member emits a single
//! `crypto.open` runtime helper whose body is the first (five-argument) overload.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled source
//! (before the member bodies), in the order `mod.rs` calls the helpers. Body
//! byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_openSealed(cipher AS SymmetricCipher, key AS List OF Byte, nonce AS List OF Byte, sealed AS Sealed, aad AS List OF Byte) AS List OF Byte
  LET ct AS List OF Byte = sealed.ciphertext
  LET tg AS List OF Byte = sealed.tag
  RETURN crypto::open(cipher, key, nonce, ct, tg, aad)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_openSealed", BODY));
}
