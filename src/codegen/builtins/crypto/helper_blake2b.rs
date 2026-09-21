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
  LET none AS List OF Byte = []
  RETURN __crypto_blake2b3(none, data, none, outLen)
END FUNC

' Unkeyed BLAKE2b over the concatenation `pre` ‖ `mid` ‖ `post`, without building it:
' a block lying wholly inside `mid` is compressed where it lies, and only a block
' straddling a boundary, and the padded final block, are gathered (plan-142-E).
FUNC __crypto_blake2b3(pre AS List OF Byte, mid AS List OF Byte, post AS List OF Byte, outLen AS Integer) AS List OF Byte
  MUT h AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 8
    h = collections::append(h, collections::get(__CRYPTO_IV512, i))
    i = i + 1
  END WHILE
  h = collections::set(h, 0, bits::bxor(collections::get(h, 0), bits::bxor(16842752, outLen)))
  LET a AS Integer = len(pre)
  LET b AS Integer = a + len(mid)
  LET n AS Integer = b + len(post)
  MUT off AS Integer = 0
  MUT t AS Integer = 0
  WHILE n - off > 128
    t = t + 128
    IF off >= a AND off + 128 <= b THEN
      h = __crypto_blake2bCompress(h, mid, off - a, t, FALSE)
    ELSE
      h = __crypto_blake2bCompress(h, __crypto_gather3(pre, mid, post, off, off + 128), 0, t, FALSE)
    END IF
    off = off + 128
  END WHILE
  MUT tail AS List OF Byte = __crypto_gather3(pre, mid, post, off, n)
  t = t + n - off
  WHILE len(tail) < 128
    tail = collections::append(tail, toByte(0))
  END WHILE
  h = __crypto_blake2bCompress(h, tail, 0, t, TRUE)
  MUT out AS List OF Byte = []
  i = 0
  WHILE i < outLen
    out = collections::append(out, toByte(bits::band(bits::sr(collections::get(h, i / 8), (i MOD 8) * 8), 255)))
    i = i + 1
  END WHILE
  RETURN out
END FUNC

' Bytes [start, stop) of `pre` ‖ `mid` ‖ `post`; never more than one block.
FUNC __crypto_gather3(pre AS List OF Byte, mid AS List OF Byte, post AS List OF Byte, start AS Integer, stop AS Integer) AS List OF Byte
  LET a AS Integer = len(pre)
  LET b AS Integer = a + len(mid)
  MUT out AS List OF Byte = []
  MUT i AS Integer = start
  WHILE i < stop
    IF i < a THEN
      out = collections::append(out, collections::get(pre, i))
    ELSE
      IF i < b THEN
        out = collections::append(out, collections::get(mid, i - a))
      ELSE
        out = collections::append(out, collections::get(post, i - b))
      END IF
    END IF
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
