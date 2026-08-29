//! `__crypto_shaBlockSize` — shared private helper for the `crypto` package.
//!
//! The hash-generic HMAC block size `B` for a `crypto::Hash`: SHA-1/SHA-224/SHA-256
//! hash in 64-byte blocks, SHA-384/SHA-512 in 128-byte blocks. `__crypto_hmac` keys and pads to
//! this width, so the construction stays written over an abstract hash `H` — a future
//! `Hash` variant needs only one new arm here.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Hash-generic HMAC block size B: 64 for SHA-1/224/256, 128 for SHA-384/512.
FUNC __crypto_shaBlockSize(algo AS Hash) AS Integer
  IF algo = Hash.SHA2_384 THEN
    RETURN 128
  END IF
  IF algo = Hash.SHA2_512 THEN
    RETURN 128
  END IF
  RETURN 64
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_shaBlockSize", BODY));
}
