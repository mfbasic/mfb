//! `__crypto_gcmGhashData` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Build the GHASH input: aad || pad16 || ct || pad16 || len(aad)*8 || len(ct)*8.
FUNC __crypto_gcmGhashData(aad AS List OF Byte, ciphertext AS List OF Byte) AS List OF Byte
  MUT data AS List OF Byte = __crypto_pad16(aad)
  data = __crypto_concat(data, __crypto_pad16(ciphertext))
  data = __crypto_concat(data, __crypto_be64(len(aad) * 8))
  data = __crypto_concat(data, __crypto_be64(len(ciphertext) * 8))
  RETURN data
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gcmGhashData", BODY));
}
