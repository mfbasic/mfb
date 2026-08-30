//! `__crypto_sha3_512_bytes` — shared private helper for the `crypto` package.
//!
//! SHA3-512 (FIPS 202 §6.1): the Keccak sponge at rate 576 bits (9 lanes,
//! capacity 1024), domain suffix `0x06`, 64-byte digest.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_sha3_512_bytes(data AS List OF Byte) AS List OF Byte
  RETURN __crypto_keccakSponge(data, 9, 6, 64)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha3_512_bytes", BODY));
}
