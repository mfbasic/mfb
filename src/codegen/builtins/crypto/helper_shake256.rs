//! `__crypto_shake256` — shared private helper for the `crypto` package.
//!
//! SHAKE256 (FIPS 202 §6.2): the Keccak sponge at rate 1088 bits (17 lanes,
//! capacity 512) with the XOF domain suffix `0x1f`, squeezed to any requested
//! `length`. Backs the public `crypto::shake256` member and is the Ed448 hash
//! (RFC 8032 §5.2 uses SHAKE256 with 114- and 57-byte outputs).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' SHAKE256 XOF (FIPS 202 §6.2): `length` output bytes, any length >= 1.
FUNC __crypto_shake256(data AS List OF Byte, length AS Integer) AS List OF Byte
  IF length < 1 THEN
    FAIL error(77050002, "shake256 length out of range")
  END IF
  RETURN __crypto_keccakSponge(data, 17, 31, length)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_shake256", BODY));
}
