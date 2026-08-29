//! `__crypto_keccakRhoTable` — shared private helper for the `crypto` package.
//!
//! The rho rotation offset `r[x][y]` (FIPS 202 §3.2.2, Table 2) of every lane,
//! listed by lane index `x + 5y`: row y=0 first (`0 1 62 28 27`), then y=1
//! (`36 44 6 55 20`), y=2 (`3 10 43 25 39`), y=3 (`41 45 15 21 8`), y=4
//! (`18 2 61 56 14`). Public constants; indexed only by the round's loop counter.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Keccak rho offsets by lane index x + 5y (FIPS 202 Table 2).
FUNC __crypto_keccakRhoTable() AS List OF Integer
  RETURN [0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56, 14]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_keccakRhoTable", BODY));
}
