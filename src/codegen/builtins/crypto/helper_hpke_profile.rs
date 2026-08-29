//! `__crypto_hpke{KemId,KdfId,KdfHash,AeadId,Aead,Nenc,Nsecret,KemSuiteId,SuiteId}`
//! — shared private helpers for the `crypto` package (registered as one chunk).
//!
//! The RFC 9180 ciphersuite profile behind each `AsymmetricCipher` value, read by
//! explicit property rather than ordinal arithmetic (plan-109-F's rule): the KEM
//! id and its `Nenc`/`Nsecret`, the KDF id and its `Hash`, the AEAD id and its
//! `SymmetricCipher`, and the two encoded suite ids (`"KEM" ‖ kem_id` for DHKEM,
//! `"HPKE" ‖ kem_id ‖ kdf_id ‖ aead_id` for the key schedule). Registry values:
//! DHKEM(X25519, HKDF-SHA256) = 0x0020 (`Nenc` 32, `Nsecret` 32); HKDF-SHA256 =
//! 0x0001; AES-256-GCM = 0x0002; ChaCha20Poly1305 = 0x0003.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 KEM id of a suite: 0x0020 DHKEM(X25519, HKDF-SHA256).
FUNC __crypto_hpkeKemId(cipher AS AsymmetricCipher) AS Integer
  RETURN 32
END FUNC
' RFC 9180 KDF id of a suite: 0x0001 HKDF-SHA256.
FUNC __crypto_hpkeKdfId(cipher AS AsymmetricCipher) AS Integer
  RETURN 1
END FUNC
' The `Hash` behind the suite's KDF (and its DHKEM's KDF, which coincide here).
FUNC __crypto_hpkeKdfHash(cipher AS AsymmetricCipher) AS Hash
  RETURN Hash.SHA2_256
END FUNC
' RFC 9180 AEAD id of a suite: 0x0002 AES-256-GCM, 0x0003 ChaCha20Poly1305.
FUNC __crypto_hpkeAeadId(cipher AS AsymmetricCipher) AS Integer
  IF cipher = AsymmetricCipher.Ed25519_AES256GCM THEN
    RETURN 2
  END IF
  RETURN 3
END FUNC
' The package AEAD behind the suite's AEAD id.
FUNC __crypto_hpkeAead(cipher AS AsymmetricCipher) AS SymmetricCipher
  IF cipher = AsymmetricCipher.Ed25519_AES256GCM THEN
    RETURN SymmetricCipher.AES256GCM
  END IF
  RETURN SymmetricCipher.CHACHA20POLY1305
END FUNC
' Nenc: the encapsulated-key length of the suite's KEM (32 for X25519).
FUNC __crypto_hpkeNenc(cipher AS AsymmetricCipher) AS Integer
  RETURN 32
END FUNC
' Nsecret: the KEM shared-secret length (32 for DHKEM(X25519, HKDF-SHA256)).
FUNC __crypto_hpkeNsecret(cipher AS AsymmetricCipher) AS Integer
  RETURN 32
END FUNC
' DHKEM suite_id = "KEM" || I2OSP(kem_id, 2).
FUNC __crypto_hpkeKemSuiteId(cipher AS AsymmetricCipher) AS List OF Byte
  RETURN __crypto_concat(strings::toBytes("KEM"), __crypto_hpkeI2osp2(__crypto_hpkeKemId(cipher)))
END FUNC
' HPKE suite_id = "HPKE" || I2OSP(kem_id, 2) || I2OSP(kdf_id, 2) || I2OSP(aead_id, 2).
FUNC __crypto_hpkeSuiteId(cipher AS AsymmetricCipher) AS List OF Byte
  MUT id AS List OF Byte = strings::toBytes("HPKE")
  id = __crypto_concat(id, __crypto_hpkeI2osp2(__crypto_hpkeKemId(cipher)))
  id = __crypto_concat(id, __crypto_hpkeI2osp2(__crypto_hpkeKdfId(cipher)))
  id = __crypto_concat(id, __crypto_hpkeI2osp2(__crypto_hpkeAeadId(cipher)))
  RETURN id
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hpkeProfile", BODY));
}
