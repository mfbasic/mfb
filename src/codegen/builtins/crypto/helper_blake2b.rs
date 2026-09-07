//! `__crypto_blake2b` — shared private helper for the `crypto` package.
//!
//! Unkeyed BLAKE2b (RFC 7693) at any output length from 1 to 64 bytes. Argon2 is
//! the only caller; the initialization vector is SHA-512's, which BLAKE2b shares, so
//! this reuses `__CRYPTO_IV512` rather than carrying a second copy of it.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT crypto
IMPORT bits
IMPORT collections

' Unkeyed BLAKE2b (RFC 7693) over `data`, producing `outLen` bytes (1..64).
FUNC __crypto_blake2b(data AS List OF Byte, outLen AS Integer) AS List OF Byte
  MUT h AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 8
    h = collections::append(h, collections::get(__CRYPTO_IV512, i))
    i = i + 1
  END WHILE
  h = collections::set(h, 0, bits::bxor(collections::get(h, 0), bits::bxor(16842752, outLen)))
  LET n AS Integer = len(data)
  MUT off AS Integer = 0
  MUT t AS Integer = 0
  WHILE n - off > 128
    t = t + 128
    h = __crypto_blake2bCompress(h, __crypto_slice(data, off, off + 128), t, FALSE)
    off = off + 128
  END WHILE
  MUT tail AS List OF Byte = __crypto_slice(data, off, n)
  t = t + n - off
  WHILE len(tail) < 128
    tail = collections::append(tail, toByte(0))
  END WHILE
  h = __crypto_blake2bCompress(h, tail, t, TRUE)
  MUT out AS List OF Byte = []
  i = 0
  WHILE i < outLen
    out = collections::append(out, toByte(bits::band(bits::sr(collections::get(h, i / 8), (i MOD 8) * 8), 255)))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_blake2b",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
