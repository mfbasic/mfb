//! `__crypto_keccakF` — shared private helper for the `crypto` package.
//!
//! The Keccak-f[1600] permutation (FIPS 202 §3.3): 24 fixed rounds of
//! `__crypto_keccakRound` with the iota constants `__CRYPTO_KECCAK_RC`. The round
//! count is a constant, so the permutation's control flow never depends on the
//! state.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Keccak-f[1600]: 24 rounds over a 25-lane (x + 5y) state of 64-bit lanes.
FUNC __crypto_keccakF(state AS List OF Integer) AS List OF Integer
  MUT a AS List OF Integer = state
  MUT round AS Integer = 0
  WHILE round < 24
    a = __crypto_keccakRound(a, collections::get(__CRYPTO_KECCAK_RC, round))
    round = round + 1
  END WHILE
  RETURN a
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_keccakF", BODY));
}
