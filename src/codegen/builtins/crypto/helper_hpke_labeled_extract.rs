//! `__crypto_hpkeLabeledExtract` — shared private helper for the `crypto` package.
//!
//! RFC 9180 §4 `LabeledExtract(salt, label, ikm)` = `HKDF-Extract(salt,
//! "HPKE-v1" ‖ suite_id ‖ label ‖ ikm)`, i.e. one HMAC under `salt` keyed by the
//! selected `Hash`. An empty `salt` is HKDF's default (`HashLen` zero bytes);
//! `__crypto_hmac` zero-pads its key to the block, which is that same key.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 LabeledExtract: HMAC(salt, "HPKE-v1" || suiteId || label || ikm).
FUNC __crypto_hpkeLabeledExtract(algo AS Hash, suiteId AS List OF Byte, salt AS List OF Byte, label AS String, ikm AS List OF Byte) AS List OF Byte
  MUT labeled AS List OF Byte = strings::toBytes("HPKE-v1")
  labeled = __crypto_concat(labeled, suiteId)
  labeled = __crypto_concat(labeled, strings::toBytes(label))
  labeled = __crypto_concat(labeled, ikm)
  RETURN __crypto_hmac(algo, salt, labeled)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hpkeLabeledExtract", BODY));
}
