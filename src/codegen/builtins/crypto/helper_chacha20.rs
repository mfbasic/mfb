//! `__crypto_chacha20` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' ChaCha20 stream: XOR `data` with the keystream starting at `counter`.
FUNC __crypto_chacha20(key AS List OF Byte, nonce AS List OF Byte, counter AS Integer, data AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = []
  LET n AS Integer = len(data)
  MUT offset AS Integer = 0
  MUT block AS Integer = counter
  WHILE offset < n
    LET ks AS List OF Byte = __crypto_chachaBlock(key, nonce, block)
    MUT j AS Integer = 0
    WHILE j < 64 AND (offset + j) < n
      LET p AS Integer = toInt(collections::get(data, offset + j))
      LET k AS Integer = toInt(collections::get(ks, j))
      out = collections::append(out, toByte(bits::bxor(p, k)))
      j = j + 1
    END WHILE
    offset = offset + 64
    block = block + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_chacha20", BODY));
}
