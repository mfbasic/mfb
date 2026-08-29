//! `__crypto_encrypt` — shared private helper for the `crypto` package.
//!
//! The pure-MFB core behind `crypto::encrypt(cipher, recipientPublicKey, data,
//! aad)`: RFC 9180 HPKE single-shot base-mode `Seal` — the recipient's Ed25519 or
//! Ed448 public key is mapped to the suite's KEM curve (`__crypto_hpkeRecipientPub`),
//! a fresh ephemeral KEM key pair (`Nenc` bytes) is drawn from `crypto::randomBytes`, and
//! `__crypto_hpkeSealWith` performs `Encap`, the key schedule, and the AEAD seal.
//! The wire value is `enc ‖ ct` (RFC 9180 §6.1, with `ct` carrying the AEAD tag).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 HPKE base-mode Seal to an Ed25519/Ed448 recipient: enc || ct (ct includes the tag).
FUNC __crypto_encrypt(cipher AS AsymmetricCipher, recipientPublicKey AS List OF Byte, data AS List OF Byte, aad AS List OF Byte) AS List OF Byte
  LET pkR AS List OF Byte = __crypto_hpkeRecipientPub(cipher, recipientPublicKey)
  LET skE AS List OF Byte = crypto::randomBytes(__crypto_hpkeNenc(cipher))
  RETURN __crypto_hpkeSealWith(cipher, pkR, skE, data, aad)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_encrypt", BODY));
}
