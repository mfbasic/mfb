//! `__crypto_hpkeSealWith` — shared private helper for the `crypto` package.
//!
//! The deterministic core of `crypto::encrypt`: RFC 9180 single-shot base-mode
//! `Seal` with a CALLER-SUPPLIED ephemeral KEM private key `skE` — `Encap`
//! (`enc = pkE`, `dh = DH(skE, pkR)`, `shared_secret = ExtractAndExpand(dh,
//! enc ‖ pkR)`), the base key schedule with empty `info`, and one AEAD seal at
//! sequence number 0 (nonce = `base_nonce`), returning `enc ‖ ct` where `ct` is
//! the AEAD ciphertext followed by its 16-byte tag. An all-zero DH output (a
//! low-order `pkR`) fails closed with `ErrInvalidArgument`. `crypto::encrypt`
//! always calls this with a fresh random `skE`; the seam exists so the
//! construction is a pure function of its inputs (it is a private helper, so no
//! user program can supply `skE`).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 one-shot base-mode Seal with an explicit ephemeral KEM key: enc || ct || tag.
FUNC __crypto_hpkeSealWith(cipher AS AsymmetricCipher, pkR AS List OF Byte, skE AS List OF Byte, data AS List OF Byte, aad AS List OF Byte) AS List OF Byte
  LET enc AS List OF Byte = __crypto_hpkeDh(cipher, skE, __crypto_hpkeBase(cipher))
  LET dh AS List OF Byte = __crypto_hpkeDh(cipher, skE, pkR)
  IF __crypto_isAllZero(dh) THEN
    FAIL error(77050002, "recipient public key is a low-order point")
  END IF
  LET sharedSecret AS List OF Byte = __crypto_hpkeExtractAndExpand(cipher, dh, __crypto_concat(enc, pkR))
  LET kn AS List OF Byte = __crypto_hpkeKeySchedule(cipher, sharedSecret, [])
  LET key AS List OF Byte = __crypto_slice(kn, 0, 32)
  LET nonce AS List OF Byte = __crypto_slice(kn, 32, 44)
  LET sealed AS Sealed = crypto::seal(__crypto_hpkeAead(cipher), key, nonce, data, aad)
  MUT box AS List OF Byte = __crypto_concat(enc, sealed.ciphertext)
  box = __crypto_concat(box, sealed.tag)
  RETURN box
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hpkeSealWith", BODY));
}
