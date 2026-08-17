//! `__crypto_sha2_32` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' SHA-256 / SHA-224 core: process `data` from `iv`, emit `outBytes` digest bytes.
FUNC __crypto_sha2_32(data AS List OF Byte, iv AS List OF Integer, outBytes AS Integer) AS List OF Byte
  LET msg AS List OF Byte = __crypto_pad512(data)
  MUT h0 AS Integer = collections::get(iv, 0)
  MUT h1 AS Integer = collections::get(iv, 1)
  MUT h2 AS Integer = collections::get(iv, 2)
  MUT h3 AS Integer = collections::get(iv, 3)
  MUT h4 AS Integer = collections::get(iv, 4)
  MUT h5 AS Integer = collections::get(iv, 5)
  MUT h6 AS Integer = collections::get(iv, 6)
  MUT h7 AS Integer = collections::get(iv, 7)
  LET total AS Integer = len(msg)
  MUT base AS Integer = 0
  WHILE base < total
    LET w AS List OF Integer = __crypto_sha256Schedule(msg, base)
    MUT a AS Integer = h0
    MUT b AS Integer = h1
    MUT c AS Integer = h2
    MUT d AS Integer = h3
    MUT e AS Integer = h4
    MUT f AS Integer = h5
    MUT g AS Integer = h6
    MUT h AS Integer = h7
    MUT t AS Integer = 0
    WHILE t < 64
      LET t1a AS Integer = __crypto_add32(h, __crypto_bsig1(e))
      LET t1b AS Integer = __crypto_add32(__crypto_ch32(e, f, g), collections::get(__CRYPTO_K256, t))
      LET t1c AS Integer = __crypto_add32(t1b, collections::get(w, t))
      LET t1 AS Integer = __crypto_add32(t1a, t1c)
      LET t2 AS Integer = __crypto_add32(__crypto_bsig0(a), __crypto_maj32(a, b, c))
      h = g
      g = f
      f = e
      e = __crypto_add32(d, t1)
      d = c
      c = b
      b = a
      a = __crypto_add32(t1, t2)
      t = t + 1
    END WHILE
    h0 = __crypto_add32(h0, a)
    h1 = __crypto_add32(h1, b)
    h2 = __crypto_add32(h2, c)
    h3 = __crypto_add32(h3, d)
    h4 = __crypto_add32(h4, e)
    h5 = __crypto_add32(h5, f)
    h6 = __crypto_add32(h6, g)
    h7 = __crypto_add32(h7, h)
    base = base + 64
  END WHILE
  MUT digest AS List OF Byte = []
  digest = __crypto_appendBeWord(digest, h0)
  digest = __crypto_appendBeWord(digest, h1)
  digest = __crypto_appendBeWord(digest, h2)
  digest = __crypto_appendBeWord(digest, h3)
  digest = __crypto_appendBeWord(digest, h4)
  digest = __crypto_appendBeWord(digest, h5)
  digest = __crypto_appendBeWord(digest, h6)
  digest = __crypto_appendBeWord(digest, h7)
  RETURN __crypto_truncate(digest, outBytes)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha2_32", BODY));
}
