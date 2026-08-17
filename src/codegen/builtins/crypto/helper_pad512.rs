//! `__crypto_pad512` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Pad `data` to the SHA-2/SHA-256 512-bit block boundary (RFC 6234 §4.1).
FUNC __crypto_pad512(data AS List OF Byte) AS List OF Byte
  MUT msg AS List OF Byte = __crypto_copyBytes(data)
  LET origLen AS Integer = len(data)
  LET bitLen AS Integer = origLen * 8
  msg = collections::append(msg, toByte(128))
  WHILE (len(msg) MOD 64) <> 56
    msg = collections::append(msg, toByte(0))
  END WHILE
  MUT pos AS Integer = 0
  WHILE pos < 8
    LET shift AS Integer = 56 - pos * 8
    LET b AS Integer = bits::band(bits::sr(bitLen, shift), 255)
    msg = collections::append(msg, toByte(b))
    pos = pos + 1
  END WHILE
  RETURN msg
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pad512", BODY));
}
