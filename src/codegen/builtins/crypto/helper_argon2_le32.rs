//! `__crypto_argon2Le32` — shared private helper for the `crypto` package.
//!
//! Argon2's `LE32`: append a 32-bit value to a byte list, little end first.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT bits
IMPORT collections

' Append `v` to `out` as four little-endian bytes (Argon2's LE32).
FUNC __crypto_argon2Le32(out AS List OF Byte, v AS Integer) AS List OF Byte
  MUT result AS List OF Byte = out
  MUT i AS Integer = 0
  WHILE i < 4
    result = collections::append(result, toByte(bits::band(bits::sr(v, i * 8), 255)))
    i = i + 1
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2Le32",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
