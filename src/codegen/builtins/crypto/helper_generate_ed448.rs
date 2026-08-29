//! `__crypto_generateEd448` — shared private helper for the `crypto` package.
//!
//! Ed448 keypair generation: a random 57-byte seed as the private key and its
//! RFC 8032 public key. Called by the `crypto::generate(Certificate.Ed448)`
//! `AbiFunction` ordinal dispatch, like `__crypto_generateEd25519`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_generateEd448() AS KeyPair
  LET seed AS List OF Byte = crypto::randomBytes(57)
  LET pub AS List OF Byte = __crypto_ed448Public(seed)
  RETURN KeyPair[seed, pub]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_generateEd448", BODY));
}
