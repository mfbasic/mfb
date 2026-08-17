//! `__crypto_aesAddRoundKey` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' XOR the 16-byte round key at word `round` into `state`.
FUNC __crypto_aesAddRoundKey(state AS List OF Byte, roundKeys AS List OF Byte, round AS Integer) AS List OF Byte
  MUT s AS List OF Byte = state
  LET base AS Integer = round * 16
  MUT i AS Integer = 0
  WHILE i < 16
    LET v AS Integer = bits::bxor(toInt(collections::get(s, i)), toInt(collections::get(roundKeys, base + i)))
    s = collections::set(s, i, toByte(v))
    i = i + 1
  END WHILE
  RETURN s
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_aesAddRoundKey", BODY));
}
