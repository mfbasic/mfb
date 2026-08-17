//! `__CRYPTO_K256` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The round-constant and initial-value tables are computed once at program
' start (module globals), not rebuilt per hash call — hashing is on the hot path
' of HMAC/HKDF/PBKDF2, where a per-call rebuild dominated the cost.
LET __CRYPTO_K256 AS List OF Integer = __crypto_sha256Ktable()"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_k256", BODY));
}
