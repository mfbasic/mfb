//! `__crypto_aesSubBytes` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __crypto_aesSubBytes(state AS List OF Byte) AS List OF Byte
  MUT s AS List OF Byte = state
  MUT i AS Integer = 0
  WHILE i < 16
    s = collections::set(s, i, toByte(__crypto_aesSub(toInt(collections::get(s, i)))))
    i = i + 1
  END WHILE
  RETURN s
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aesSubBytes", BODY));
}
