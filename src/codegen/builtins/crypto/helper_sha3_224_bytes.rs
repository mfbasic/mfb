//! `__crypto_sha3_224_bytes` — shared private helper for the `crypto` package.
//!
//! SHA3-224 (FIPS 202 §6.1): the Keccak sponge at rate 1152 bits (18 lanes,
//! capacity 448), domain suffix `0x06`, 28-byte digest.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sha3_224_bytes(data AS List OF Byte) AS List OF Byte
  RETURN __crypto_keccakSponge(data, 18, 6, 28)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha3_224_bytes", BODY));
}
