//! `__crypto_decrypt` — shared private helper for the `crypto` package.
//!
//! The pure-MFB core behind `crypto::decrypt(cipher, recipientPrivateKey, box,
//! aad)`: RFC 9180 HPKE single-shot base-mode `Open`. The box must be at least
//! `Nenc + 16` bytes (`ErrInvalidArgument` otherwise); `enc` is its first `Nenc`
//! bytes and the rest is the AEAD ciphertext ending in the 16-byte tag. The
//! recipient's Ed25519 or Ed448 seed is mapped to the KEM private key
//! (`__crypto_hpkeRecipientPriv`), `Decap` recomputes `dh = DH(skR, enc)` — an
//! all-zero output (a low-order `enc`) fails closed with `ErrInvalidArgument` —
//! and `shared_secret = ExtractAndExpand(dh, enc ‖ pkR)`; the same key schedule
//! yields the key and base nonce, and the AEAD open at sequence 0 either returns
//! the plaintext or raises `ErrAuthenticationFailed`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 HPKE base-mode Open of enc || ct by an Ed25519/Ed448 recipient.
FUNC __crypto_decrypt(cipher AS AsymmetricCipher, recipientPrivateKey AS List OF Byte, box AS List OF Byte, aad AS List OF Byte) AS List OF Byte
  LET nenc AS Integer = __crypto_hpkeNenc(cipher)
  LET total AS Integer = len(box)
  IF total < nenc + 16 THEN
    FAIL error(77050002, "crypto::decrypt box shorter than the enc || tag overhead")
  END IF
  LET skR AS List OF Byte = __crypto_hpkeRecipientPriv(cipher, recipientPrivateKey)
  LET enc AS List OF Byte = __crypto_slice(box, 0, nenc)
  LET ct AS List OF Byte = __crypto_slice(box, nenc, total - 16)
  LET tag AS List OF Byte = __crypto_slice(box, total - 16, total)
  LET pkR AS List OF Byte = __crypto_hpkeDh(cipher, skR, __crypto_hpkeBase(cipher))
  LET dh AS List OF Byte = __crypto_hpkeDh(cipher, skR, enc)
  IF __crypto_isAllZero(dh) THEN
    FAIL error(77050002, "encapsulated key is a low-order point")
  END IF
  LET sharedSecret AS List OF Byte = __crypto_hpkeExtractAndExpand(cipher, dh, __crypto_concat(enc, pkR))
  LET kn AS List OF Byte = __crypto_hpkeKeySchedule(cipher, sharedSecret, [])
  LET key AS List OF Byte = __crypto_slice(kn, 0, 32)
  LET nonce AS List OF Byte = __crypto_slice(kn, 32, 44)
  RETURN crypto::open(__crypto_hpkeAead(cipher), key, nonce, ct, tag, aad)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_decrypt", BODY));
}
