//! `__crypto_hkdfExpand` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C6: one HKDF-Expand ladder parameterized by the HMAC primitive, instead
' of byte-identical __crypto_hkdfExpand256/512 that differed only in the hash.
FUNC __crypto_hkdfExpand(prk AS List OF Byte, info AS List OF Byte, length AS Integer, hmac AS FUNC(List OF Byte, List OF Byte) AS List OF Byte) AS List OF Byte
  MUT okm AS List OF Byte = []
  MUT prev AS List OF Byte = []
  MUT counter AS Integer = 1
  WHILE len(okm) < length
    MUT block AS List OF Byte = __crypto_concat(prev, info)
    block = collections::append(block, toByte(counter))
    prev = hmac(prk, block)
    okm = __crypto_concat(okm, prev)
    counter = counter + 1
  END WHILE
  RETURN __crypto_truncate(okm, length)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_hkdfExpand", BODY));
}
