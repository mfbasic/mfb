//! `__crypto_hkdf` — shared private helper for the `crypto` package.
//!
//! The hash-generic HKDF (RFC 5869) Extract+Expand, written over the hash-generic
//! `__crypto_hmac` and the digest length `__crypto_shaOutputLen`. It reuses the
//! already hash-agnostic `__crypto_hkdfExpand` ladder, supplying it a hash-generic HMAC
//! closure that binds the `Hash` selector — so the same construction serves every `Hash`
//! variant. It is the single MFB body behind the unified `crypto::hkdf(Hash, …)` member.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Hash-generic HKDF-Extract + Expand (RFC 5869). Default salt and the 255*L output
' ceiling come from the digest length L of the selected `Hash`.
FUNC __crypto_hkdf(algo AS Hash, ikm AS List OF Byte, salt AS List OF Byte, info AS List OF Byte, length AS Integer) AS List OF Byte
  LET outLen AS Integer = __crypto_shaOutputLen(algo)
  IF length < 1 OR length > (255 * outLen) THEN
    FAIL error(77050002, "hkdf length out of range")
  END IF
  MUT usedSalt AS List OF Byte = salt
  IF len(usedSalt) = 0 THEN
    usedSalt = __crypto_zeroPad([], outLen)
  END IF
  LET prk AS List OF Byte = __crypto_hmac(algo, usedSalt, ikm)
  RETURN __crypto_hkdfExpand(prk, info, length, LAMBDA(mk AS List OF Byte, md AS List OF Byte) -> __crypto_hmac(algo, mk, md))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hkdf", BODY));
}
