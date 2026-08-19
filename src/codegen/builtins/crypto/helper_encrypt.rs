//! `__crypto_encrypt` — shared private helper for the `crypto` package.
//!
//! The pure-MFB core behind `crypto::encrypt(cipher, recipientPublicKey, data[, aad])`:
//! an X25519 sealed-box hybrid (deterministic given the random ephemeral key). It
//! converts the recipient's Ed25519 public key to X25519, generates an ephemeral
//! X25519 key pair, does the Diffie–Hellman, derives a 32-byte key + 12-byte nonce
//! with `HKDF(SHA-256)` (salt = ephPub ‖ recipX, info = "mfb-box-v1" ‖ ordinal),
//! seals `data` with the suite's AEAD, and returns the self-contained box
//! `ephPub(32) ‖ ciphertext ‖ tag(16)`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' X25519 sealed-box encrypt. recipientPublicKey is a 32-byte Ed25519 public key.
FUNC __crypto_encrypt(cipher AS AsymmetricCipher, recipientPublicKey AS List OF Byte, data AS List OF Byte, aad AS List OF Byte) AS List OF Byte
  LET recipX AS List OF Byte = __crypto_ed25519PubToX25519(recipientPublicKey)
  LET eph AS KeyPair = crypto::generate(Certificate.X25519)
  LET ephPub AS List OF Byte = eph.publicKey
  LET ephPriv AS List OF Byte = eph.privateKey
  LET dh AS List OF Byte = __crypto_x25519(ephPriv, recipX)
  LET salt AS List OF Byte = __crypto_concat(ephPub, recipX)
  LET info AS List OF Byte = __crypto_asymInfo(cipher)
  LET okm AS List OF Byte = __crypto_hkdf(Hash.SHA256, dh, salt, info, 44)
  LET k AS List OF Byte = __crypto_slice(okm, 0, 32)
  LET nonce AS List OF Byte = __crypto_slice(okm, 32, 44)
  LET aead AS SymmetricCipher = __crypto_asymAead(cipher)
  LET sealed AS Sealed = crypto::seal(aead, k, nonce, data, aad)
  LET ct AS List OF Byte = sealed.ciphertext
  LET tag AS List OF Byte = sealed.tag
  MUT box AS List OF Byte = __crypto_concat(ephPub, ct)
  box = __crypto_concat(box, tag)
  RETURN box
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_encrypt", BODY));
}
