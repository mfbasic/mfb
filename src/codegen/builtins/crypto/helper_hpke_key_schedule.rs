//! `__crypto_hpkeKeySchedule` — shared private helper for the `crypto` package.
//!
//! RFC 9180 §5.1 `KeySchedule` for mode_base (0x00) with no PSK:
//! `psk_id_hash = LabeledExtract("", "psk_id_hash", "")`,
//! `info_hash = LabeledExtract("", "info_hash", info)`,
//! `key_schedule_context = mode ‖ psk_id_hash ‖ info_hash`,
//! `secret = LabeledExtract(shared_secret, "secret", "")`,
//! `key = LabeledExpand(secret, "key", context, Nk)`,
//! `base_nonce = LabeledExpand(secret, "base_nonce", context, Nn)`, all under the
//! HPKE suite id. Returns `key ‖ base_nonce` (32 + 12 bytes for both AEADs). The
//! exporter secret is not derived — the one-shot API exposes no exporter.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 base-mode KeySchedule (no PSK): returns key(32) || base_nonce(12).
FUNC __crypto_hpkeKeySchedule(cipher AS AsymmetricCipher, sharedSecret AS List OF Byte, info AS List OF Byte) AS List OF Byte
  LET algo AS Hash = __crypto_hpkeKdfHash(cipher)
  LET suite AS List OF Byte = __crypto_hpkeSuiteId(cipher)
  LET pskIdHash AS List OF Byte = __crypto_hpkeLabeledExtract(algo, suite, [], "psk_id_hash", [])
  LET infoHash AS List OF Byte = __crypto_hpkeLabeledExtract(algo, suite, [], "info_hash", info)
  MUT context AS List OF Byte = []
  context = collections::append(context, toByte(0))
  context = __crypto_concat(context, pskIdHash)
  context = __crypto_concat(context, infoHash)
  LET secret AS List OF Byte = __crypto_hpkeLabeledExtract(algo, suite, sharedSecret, "secret", [])
  LET key AS List OF Byte = __crypto_hpkeLabeledExpand(algo, suite, secret, "key", context, 32)
  LET baseNonce AS List OF Byte = __crypto_hpkeLabeledExpand(algo, suite, secret, "base_nonce", context, 12)
  RETURN __crypto_concat(key, baseNonce)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hpkeKeySchedule", BODY));
}
