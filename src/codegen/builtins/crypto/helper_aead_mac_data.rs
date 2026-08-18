//! `__crypto_aeadMacData` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The Poly1305 authentication input: aad || pad16 || ct || pad16 || len(aad) || len(ct).
FUNC __crypto_aeadMacData(aad AS List OF Byte, ciphertext AS List OF Byte) AS List OF Byte
  MUT macData AS List OF Byte = __crypto_pad16(aad)
  macData = __crypto_concat(macData, __crypto_pad16(ciphertext))
  macData = __crypto_concat(macData, __crypto_le64(len(aad)))
  macData = __crypto_concat(macData, __crypto_le64(len(ciphertext)))
  RETURN macData
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aeadMacData", BODY));
}
