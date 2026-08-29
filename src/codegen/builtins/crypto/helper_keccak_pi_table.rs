//! `__crypto_keccakPiTable` — shared private helper for the `crypto` package.
//!
//! The pi lane permutation (FIPS 202 §3.2.3): lane `(x, y)` moves to
//! `(y, (2x + 3y) mod 5)`. Listed as the DESTINATION index `y + 5·((2x+3y) mod 5)`
//! of each SOURCE lane index `x + 5y`, so a round writes
//! `B[PI[i]] = ROTL(A[i], RHO[i])` in one pass. Public constants; indexed only by
//! the round's loop counter.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Keccak pi destination index of each source lane x + 5y (FIPS 202 §3.2.3).
FUNC __crypto_keccakPiTable() AS List OF Integer
  RETURN [0, 10, 20, 5, 15, 16, 1, 11, 21, 6, 7, 17, 2, 12, 22, 23, 8, 18, 3, 13, 14, 24, 9, 19, 4]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_keccakPiTable", BODY));
}
