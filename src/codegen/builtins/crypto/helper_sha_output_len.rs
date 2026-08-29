//! `__crypto_shaOutputLen` — shared private helper for the `crypto` package.
//!
//! The hash-generic digest length `L` for a `crypto::Hash`: SHA1=20, SHA2_224=28,
//! SHA2_256=32, SHA2_384=48, SHA2_512=64, and the same 28/32/48/64 for
//! SHA3_224/256/384/512. `__crypto_hkdf` uses it for the all-zero default salt and the
//! `255 * L` output-length ceiling, so HKDF stays written over an abstract hash `H` — a
//! future `Hash` variant needs only one new arm here.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Hash-generic digest length L: 20 for SHA-1; 28/32/48/64 for SHA-2 and SHA-3 at 224/256/384/512.
FUNC __crypto_shaOutputLen(algo AS Hash) AS Integer
  IF algo = Hash.SHA1 THEN
    RETURN 20
  END IF
  IF algo = Hash.SHA2_224 THEN
    RETURN 28
  END IF
  IF algo = Hash.SHA2_256 THEN
    RETURN 32
  END IF
  IF algo = Hash.SHA2_384 THEN
    RETURN 48
  END IF
  IF algo = Hash.SHA3_224 THEN
    RETURN 28
  END IF
  IF algo = Hash.SHA3_256 THEN
    RETURN 32
  END IF
  IF algo = Hash.SHA3_384 THEN
    RETURN 48
  END IF
  RETURN 64
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_shaOutputLen", BODY));
}
