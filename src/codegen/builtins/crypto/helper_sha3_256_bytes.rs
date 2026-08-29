//! `__crypto_sha3_256_bytes` — shared private helper for the `crypto` package.
//!
//! SHA3-256 (FIPS 202 §6.1): the Keccak sponge at rate 1088 bits (17 lanes,
//! capacity 512), domain suffix `0x06`, 32-byte digest.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sha3_256_bytes(data AS List OF Byte) AS List OF Byte
  RETURN __crypto_keccakSponge(data, 17, 6, 32)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha3_256_bytes", BODY));
}
