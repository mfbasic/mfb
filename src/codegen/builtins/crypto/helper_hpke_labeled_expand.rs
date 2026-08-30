//! `__crypto_hpkeLabeledExpand` — shared private helper for the `crypto` package.
//!
//! RFC 9180 §4 `LabeledExpand(prk, label, info, L)` = `HKDF-Expand(prk,
//! I2OSP(L, 2) ‖ "HPKE-v1" ‖ suite_id ‖ label ‖ info, L)`, over the package's
//! hash-generic `__crypto_hkdfExpand` ladder with an HMAC closure bound to the
//! selected `Hash`. `L` is at most `255 · HashLen` (the callers ask for 32, 12,
//! or `Nsecret` ≤ 64 bytes).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' RFC 9180 LabeledExpand: HKDF-Expand(prk, I2OSP(L,2) || "HPKE-v1" || suiteId || label || info, L).
FUNC __crypto_hpkeLabeledExpand(algo AS Hash, suiteId AS List OF Byte, prk AS List OF Byte, label AS String, info AS List OF Byte, length AS Integer) AS List OF Byte
  MUT labeled AS List OF Byte = __crypto_hpkeI2osp2(length)
  labeled = __crypto_concat(labeled, strings::toBytes("HPKE-v1"))
  labeled = __crypto_concat(labeled, suiteId)
  labeled = __crypto_concat(labeled, strings::toBytes(label))
  labeled = __crypto_concat(labeled, info)
  RETURN __crypto_hkdfExpand(prk, labeled, length, LAMBDA(mk AS List OF Byte, md AS List OF Byte) -> __crypto_hmac(algo, mk, md))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hpkeLabeledExpand", BODY));
}
