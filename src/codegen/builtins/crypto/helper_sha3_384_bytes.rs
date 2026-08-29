//! `__crypto_sha3_384_bytes` — shared private helper for the `crypto` package.
//!
//! SHA3-384 (FIPS 202 §6.1): the Keccak sponge at rate 832 bits (13 lanes,
//! capacity 768), domain suffix `0x06`, 48-byte digest.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sha3_384_bytes(data AS List OF Byte) AS List OF Byte
  RETURN __crypto_keccakSponge(data, 13, 6, 48)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha3_384_bytes", BODY));
}
