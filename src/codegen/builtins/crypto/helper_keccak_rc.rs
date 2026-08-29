//! `__CRYPTO_KECCAK_RC` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Keccak-f[1600] tables, computed once at program start (module globals) like the
' SHA-2 constants: the 24 iota round constants, the rho rotation offset of each of
' the 25 lanes (index x + 5y), and the pi destination index of each lane.
LET __CRYPTO_KECCAK_RC AS List OF Integer = __crypto_keccakRcTable()"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_keccakRc", BODY));
}
