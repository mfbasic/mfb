//! `__crypto_be32` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The big-endian 4-byte block index appended to the salt in PBKDF2's F.
FUNC __crypto_be32(value AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  out = collections::append(out, toByte(bits::band(bits::sr(value, 24), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(value, 16), 255)))
  out = collections::append(out, toByte(bits::band(bits::sr(value, 8), 255)))
  out = collections::append(out, toByte(bits::band(value, 255)))
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_be32", BODY));
}
