//! `__crypto_shaDigest` — shared private helper for the `crypto` package.
//!
//! The hash-generic dispatch that turns a `crypto::Hash` selector into the matching
//! always-emitted MFB software SHA `_bytes` core. It is the single point the keyed-hash
//! constructions (`__crypto_hmac`/`__crypto_hkdf`/`__crypto_pbkdf2`) route their digest
//! through, so those stay written over an abstract hash `H` — adding a future `Hash`
//! variant is one new arm here (plus `__crypto_shaBlockSize`/`__crypto_shaOutputLen`),
//! and every construction below lights up.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Hash-generic digest dispatch: route a `Hash` selector to its SHA `_bytes` core.
FUNC __crypto_shaDigest(algo AS Hash, data AS List OF Byte) AS List OF Byte
  IF algo = Hash.SHA224 THEN
    RETURN __crypto_sha224_bytes(data)
  END IF
  IF algo = Hash.SHA256 THEN
    RETURN __crypto_sha256_bytes(data)
  END IF
  IF algo = Hash.SHA384 THEN
    RETURN __crypto_sha384_bytes(data)
  END IF
  RETURN __crypto_sha512_bytes(data)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_shaDigest", BODY));
}
