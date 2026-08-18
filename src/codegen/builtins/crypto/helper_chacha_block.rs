//! `__crypto_chachaBlock` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The 64-byte ChaCha20 keystream block for `counter`.
FUNC __crypto_chachaBlock(key AS List OF Byte, nonce AS List OF Byte, counter AS Integer) AS List OF Byte
  LET init AS List OF Integer = __crypto_chachaState(key, nonce, counter)
  MUT s AS List OF Integer = __crypto_chachaState(key, nonce, counter)
  MUT r AS Integer = 0
  WHILE r < 10
    s = __crypto_chachaQr(s, 0, 4, 8, 12)
    s = __crypto_chachaQr(s, 1, 5, 9, 13)
    s = __crypto_chachaQr(s, 2, 6, 10, 14)
    s = __crypto_chachaQr(s, 3, 7, 11, 15)
    s = __crypto_chachaQr(s, 0, 5, 10, 15)
    s = __crypto_chachaQr(s, 1, 6, 11, 12)
    s = __crypto_chachaQr(s, 2, 7, 8, 13)
    s = __crypto_chachaQr(s, 3, 4, 9, 14)
    r = r + 1
  END WHILE
  MUT out AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < 16
    LET word AS Integer = __crypto_add32(collections::get(s, i), collections::get(init, i))
    out = __crypto_appendLeWord(out, word)
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_chachaBlock", BODY));
}
