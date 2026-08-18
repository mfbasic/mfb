//! `__crypto_ed25519Public` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_ed25519Public(seed AS List OF Byte) AS List OF Byte
  LET d AS List OF Byte = __crypto_sha512_bytes(seed)
  LET a AS List OF Byte = __crypto_clampScalar(__crypto_truncate(d, 32))
  LET p AS List OF Integer = __crypto_scalarbase(a)
  RETURN __crypto_packPoint(p)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed25519Public", BODY));
}
