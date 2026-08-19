//! `__crypto_generateX25519` — shared private helper for the `crypto` package.
//!
//! X25519 keypair generation: a clamped random 32-byte scalar as the private key,
//! and its public key `X25519(scalar, basepoint u=9)` (RFC 7748 §6.1). Called by
//! the `crypto::generate(Certificate.X25519)` `AbiFunction` ordinal dispatch, the
//! same way its Ed25519 branch calls `__crypto_generateEd25519`.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_generateX25519() AS KeyPair
  LET scalar AS List OF Byte = __crypto_clampScalar(crypto::randomBytes(32))
  MUT base AS List OF Byte = []
  base = collections::append(base, toByte(9))
  MUT i AS Integer = 1
  WHILE i < 32
    base = collections::append(base, toByte(0))
    i = i + 1
  END WHILE
  LET pub AS List OF Byte = __crypto_x25519(scalar, base)
  RETURN KeyPair[scalar, pub]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_generateX25519", BODY));
}
