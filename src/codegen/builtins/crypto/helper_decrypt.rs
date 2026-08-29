//! `__crypto_decrypt` — shared private helper for the `crypto` package.
//!
//! The pure-MFB core behind `crypto::decrypt(cipher, recipientPrivateKey, box[, aad])`:
//! the inverse of `__crypto_encrypt`. It first rejects a box shorter than the 48-byte
//! `ephemeralPublicKey(32) ‖ tag(16)` overhead with `ErrInvalidArgument` (code
//! `77050002`) — without this guard the `__crypto_slice`/`collections::mid` calls below
//! would surface `ErrIndexOutOfRange` on the negative `ctLen`, not the documented
//! argument error. Then it splits the box into `ephPub(32)`,
//! ciphertext, and the trailing 16-byte tag; converts the recipient's Ed25519 seed
//! to its X25519 scalar; recovers the recipient's X25519 public key as
//! `X25519(recipXpriv, basepoint)` (which equals `ed25519PubToX25519(recipEdPub)`,
//! so the HKDF salt matches encrypt); does the Diffie–Hellman against `ephPub`;
//! re-derives the key/nonce; and `crypto::open`s the AEAD (failing closed with
//! `ErrAuthenticationFailed` on any tamper/wrong-key/wrong-aad).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' X25519 sealed-box decrypt. recipientPrivateKey is a 32-byte Ed25519 seed.
FUNC __crypto_decrypt(cipher AS AsymmetricCipher, recipientPrivateKey AS List OF Byte, box AS List OF Byte, aad AS List OF Byte) AS List OF Byte
  LET total AS Integer = len(box)
  IF total < 48 THEN
    FAIL error(77050002, "crypto::decrypt box shorter than the 48-byte ephemeralPublicKey||tag overhead")
  END IF
  LET ephPub AS List OF Byte = __crypto_slice(box, 0, 32)
  LET ctLen AS Integer = total - 48
  LET ct AS List OF Byte = __crypto_slice(box, 32, 32 + ctLen)
  LET tag AS List OF Byte = __crypto_slice(box, 32 + ctLen, total)
  LET recipXpriv AS List OF Byte = __crypto_ed25519PrivToX25519(recipientPrivateKey)
  MUT base AS List OF Byte = []
  base = collections::append(base, toByte(9))
  MUT i AS Integer = 1
  WHILE i < 32
    base = collections::append(base, toByte(0))
    i = i + 1
  END WHILE
  LET recipX AS List OF Byte = __crypto_x25519(recipXpriv, base)
  LET dh AS List OF Byte = __crypto_x25519(recipXpriv, ephPub)
  LET salt AS List OF Byte = __crypto_concat(ephPub, recipX)
  LET info AS List OF Byte = __crypto_asymInfo(cipher)
  LET okm AS List OF Byte = __crypto_hkdf(Hash.SHA2_256, dh, salt, info, 44)
  LET k AS List OF Byte = __crypto_slice(okm, 0, 32)
  LET nonce AS List OF Byte = __crypto_slice(okm, 32, 44)
  LET aead AS SymmetricCipher = __crypto_asymAead(cipher)
  RETURN crypto::open(aead, k, nonce, ct, tag, aad)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_decrypt", BODY));
}
