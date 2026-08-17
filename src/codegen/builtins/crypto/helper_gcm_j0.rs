//! `__crypto_gcmJ0` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The 16-byte counter block J0 for a 96-bit nonce: nonce || 0x00000001.
FUNC __crypto_gcmJ0(nonce AS List OF Byte) AS List OF Byte
  MUT j AS List OF Byte = __crypto_copyBytes(nonce)
  j = collections::append(j, toByte(0))
  j = collections::append(j, toByte(0))
  j = collections::append(j, toByte(0))
  j = collections::append(j, toByte(1))
  RETURN j
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_gcmJ0", BODY));
}
