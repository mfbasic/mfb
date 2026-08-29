//! `__crypto_hpke{Dh,Base,RecipientPub,RecipientPriv,ExtractAndExpand}` — shared
//! private helpers for the `crypto` package (registered as one chunk).
//!
//! The DHKEM layer of RFC 9180 §4.1 for each suite: the curve's `DH()` and base
//! point (X25519 / `u = 9` for the `Ed25519_*` suites, X448 / `u = 5` for the
//! `Ed448_*` suites), the conversion of the recipient's signing key to the KEM
//! curve (the `Ed25519ToX25519` / `Ed448ToX448` maps of `crypto::convert`, with
//! the 32-/57-byte length checks), and `ExtractAndExpand(dh, kem_context)` =
//! `LabeledExpand(LabeledExtract("", "eae_prk", dh), "shared_secret",
//! kem_context, Nsecret)` under the KEM suite id.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The suite's KEM curve DH(): X448 for the Ed448-named suites, X25519 otherwise.
FUNC __crypto_hpkeDh(cipher AS AsymmetricCipher, sk AS List OF Byte, pk AS List OF Byte) AS List OF Byte
  IF __crypto_hpkeIsX448(cipher) THEN
    RETURN __crypto_x448(sk, pk)
  END IF
  RETURN __crypto_x25519(sk, pk)
END FUNC
' The suite's KEM base point encoding (u = 5 for X448, u = 9 for X25519).
FUNC __crypto_hpkeBase(cipher AS AsymmetricCipher) AS List OF Byte
  IF __crypto_hpkeIsX448(cipher) THEN
    RETURN __crypto_x448Base()
  END IF
  MUT base AS List OF Byte = []
  base = collections::append(base, toByte(9))
  MUT i AS Integer = 1
  WHILE i < 32
    base = collections::append(base, toByte(0))
    i = i + 1
  END WHILE
  RETURN base
END FUNC
' Recipient signing public key -> KEM public key (57-byte Ed448 -> X448, 32-byte Ed25519 -> X25519).
FUNC __crypto_hpkeRecipientPub(cipher AS AsymmetricCipher, edPub AS List OF Byte) AS List OF Byte
  IF __crypto_hpkeIsX448(cipher) THEN
    IF len(edPub) <> 57 THEN
      FAIL error(77050002, "recipient public key must be a 57-byte Ed448 key")
    END IF
    RETURN __crypto_ed448PubToX448(edPub)
  END IF
  IF len(edPub) <> 32 THEN
    FAIL error(77050002, "recipient public key must be a 32-byte Ed25519 key")
  END IF
  RETURN __crypto_ed25519PubToX25519(edPub)
END FUNC
' Recipient signing private key -> KEM private key (57-byte Ed448 seed -> X448, 32-byte Ed25519 seed -> X25519).
FUNC __crypto_hpkeRecipientPriv(cipher AS AsymmetricCipher, edPriv AS List OF Byte) AS List OF Byte
  IF __crypto_hpkeIsX448(cipher) THEN
    IF len(edPriv) <> 57 THEN
      FAIL error(77050002, "recipient private key must be a 57-byte Ed448 seed")
    END IF
    RETURN __crypto_ed448PrivToX448(edPriv)
  END IF
  IF len(edPriv) <> 32 THEN
    FAIL error(77050002, "recipient private key must be a 32-byte Ed25519 seed")
  END IF
  RETURN __crypto_ed25519PrivToX25519(edPriv)
END FUNC
' DHKEM ExtractAndExpand: shared_secret from the DH output and kem_context = enc || pkR.
FUNC __crypto_hpkeExtractAndExpand(cipher AS AsymmetricCipher, dh AS List OF Byte, kemContext AS List OF Byte) AS List OF Byte
  LET algo AS Hash = __crypto_hpkeKdfHash(cipher)
  LET suite AS List OF Byte = __crypto_hpkeKemSuiteId(cipher)
  LET eaePrk AS List OF Byte = __crypto_hpkeLabeledExtract(algo, suite, [], "eae_prk", dh)
  RETURN __crypto_hpkeLabeledExpand(algo, suite, eaePrk, "shared_secret", kemContext, __crypto_hpkeNsecret(cipher))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hpkeKem", BODY));
}
