//! `__crypto_generateX448` — shared private helper for the `crypto` package.
//!
//! X448 keypair generation: a clamped random 56-byte scalar as the private key,
//! and its public key `X448(scalar, basepoint u=5)` (RFC 7748 §6.2). Called by the
//! `crypto::generate(Certificate.X448)` `AbiFunction` ordinal dispatch, the same
//! way its X25519 branch calls `__crypto_generateX25519`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_generateX448() AS KeyPair
  LET scalar AS List OF Byte = __crypto_clampScalar448(crypto::randomBytes(56))
  LET pub AS List OF Byte = __crypto_x448(scalar, __crypto_x448Base())
  RETURN KeyPair[scalar, pub]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_generateX448", BODY));
}
