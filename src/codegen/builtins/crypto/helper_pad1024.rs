//! `__crypto_pad1024` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Pad `data` to the SHA-512 1024-bit block boundary (RFC 6234 §4.2). Message
' lengths never approach 2^64 bits, so the high 64 length bits are always zero.
FUNC __crypto_pad1024(data AS List OF Byte) AS List OF Byte
  MUT msg AS List OF Byte = __crypto_copyBytes(data)
  LET bitLen AS Integer = len(data) * 8
  msg = collections::append(msg, toByte(128))
  WHILE (len(msg) MOD 128) <> 112
    msg = collections::append(msg, toByte(0))
  END WHILE
  MUT z AS Integer = 0
  WHILE z < 8
    msg = collections::append(msg, toByte(0))
    z = z + 1
  END WHILE
  msg = __crypto_appendBeWord64(msg, bitLen)
  RETURN msg
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_pad1024", BODY));
}
