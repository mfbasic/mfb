//! `__crypto_chachaState` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Build the 16-word ChaCha20 state for `key` (32 bytes), `nonce` (12 bytes),
' block `counter`.
FUNC __crypto_chachaState(key AS List OF Byte, nonce AS List OF Byte, counter AS Integer) AS List OF Integer
  MUT s AS List OF Integer = []
  s = collections::append(s, 1634760805)
  s = collections::append(s, 857760878)
  s = collections::append(s, 2036477234)
  s = collections::append(s, 1797285236)
  MUT i AS Integer = 0
  WHILE i < 8
    s = collections::append(s, __crypto_leWord(key, i * 4))
    i = i + 1
  END WHILE
  s = collections::append(s, bits::band(counter, 4294967295))
  s = collections::append(s, __crypto_leWord(nonce, 0))
  s = collections::append(s, __crypto_leWord(nonce, 4))
  s = collections::append(s, __crypto_leWord(nonce, 8))
  RETURN s
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_chachaState", BODY));
}
