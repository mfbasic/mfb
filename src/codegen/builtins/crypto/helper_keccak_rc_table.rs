//! `__crypto_keccakRcTable` — shared private helper for the `crypto` package.
//!
//! The 24 Keccak-f[1600] iota round constants `RC[0..23]` (FIPS 202 §3.2.5, the
//! `rc(t)` LFSR expanded), decoded from their big-endian hex into one full 64-bit
//! lane each — a raw two's-complement bit pattern, as `bits::` treats every
//! `Integer` (constants with bit 63 set are negative `Integer`s and never enter
//! trapping arithmetic; Keccak is XOR/AND/NOT/rotate only).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The 24 Keccak-f[1600] round constants RC[t] (FIPS 202 §3.2.5), one 64-bit lane each.
FUNC __crypto_keccakRcTable() AS List OF Integer
  LET hex AS String = "00000000000000010000000000008082800000000000808a8000000080008000000000000000808b000000008000000180000000800080818000000000008009000000000000008a00000000000000880000000080008009000000008000000a"
  LET hex2 AS String = "000000008000808b800000000000008b8000000000008089800000000000800380000000000080028000000000000080000000000000800a800000008000000a8000000080008081800000000000808000000000800000018000000080008008"
  LET raw AS List OF Byte = encoding::hexDecode(hex & hex2)
  MUT rc AS List OF Integer = []
  MUT t AS Integer = 0
  WHILE t < 24
    rc = collections::append(rc, __crypto_beWord64(raw, t * 8))
    t = t + 1
  END WHILE
  RETURN rc
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_keccakRcTable", BODY));
}
