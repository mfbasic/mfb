//! `__crypto_sha2_64` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' SHA-512 / SHA-384 core: process `data` from `iv`, emit `outBytes` digest bytes.
FUNC __crypto_sha2_64(data AS List OF Byte, iv AS List OF Integer, outBytes AS Integer) AS List OF Byte
  LET msg AS List OF Byte = __crypto_pad1024(data)
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
    LET w AS List OF Integer = __crypto_sha512Schedule(msg, base)
    MUT a AS Integer = h0
    MUT b AS Integer = h1
    MUT c AS Integer = h2
    MUT d AS Integer = h3
    MUT e AS Integer = h4
    MUT f AS Integer = h5
    MUT g AS Integer = h6
    MUT h AS Integer = h7
    MUT t AS Integer = 0
    WHILE t < 80
      LET t1a AS Integer = __crypto_add64(h, __crypto_bsig1_64(e))
      LET t1b AS Integer = __crypto_add64(__crypto_ch64(e, f, g), collections::get(__CRYPTO_K512, t))
      LET t1c AS Integer = __crypto_add64(t1b, collections::get(w, t))
      LET t1 AS Integer = __crypto_add64(t1a, t1c)
      LET t2 AS Integer = __crypto_add64(__crypto_bsig0_64(a), __crypto_maj64(a, b, c))
      h = g
      g = f
      f = e
      e = __crypto_add64(d, t1)
      d = c
      c = b
      b = a
      a = __crypto_add64(t1, t2)
      t = t + 1
    END WHILE
    h0 = __crypto_add64(h0, a)
    h1 = __crypto_add64(h1, b)
    h2 = __crypto_add64(h2, c)
    h3 = __crypto_add64(h3, d)
    h4 = __crypto_add64(h4, e)
    h5 = __crypto_add64(h5, f)
    h6 = __crypto_add64(h6, g)
    h7 = __crypto_add64(h7, h)
    base = base + 128
  END WHILE
  MUT digest AS List OF Byte = []
  digest = __crypto_appendBeWord64(digest, h0)
  digest = __crypto_appendBeWord64(digest, h1)
  digest = __crypto_appendBeWord64(digest, h2)
  digest = __crypto_appendBeWord64(digest, h3)
  digest = __crypto_appendBeWord64(digest, h4)
  digest = __crypto_appendBeWord64(digest, h5)
  digest = __crypto_appendBeWord64(digest, h6)
  digest = __crypto_appendBeWord64(digest, h7)
  RETURN __crypto_truncate(digest, outBytes)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha2_64", BODY));
}
