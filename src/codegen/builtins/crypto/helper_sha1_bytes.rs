//! `__crypto_sha1_bytes` — shared private helper for the `crypto` package.
//!
//! The SHA-1 core (FIPS 180-4 §6.1): the SHA-256 padding (`__crypto_pad512`, the two
//! share the 512-bit block and 64-bit length trailer), then 80 rounds per block over
//! five 32-bit working words with the `__crypto_sha1F`/`__crypto_sha1K` round
//! function and constant. All arithmetic is masked 32-bit (`__crypto_add32`,
//! `bits::rl32`), so no intermediate leaves `0..2^32-1`. Every loop bound and index
//! is a public counter or the public message length; nothing branches on data.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' SHA-1 core (FIPS 180-4 §6.1): 80 rounds per 512-bit block, 20-byte digest.
FUNC __crypto_sha1_bytes(data AS List OF Byte) AS List OF Byte
  LET msg AS List OF Byte = __crypto_pad512(data)
  MUT h0 AS Integer = 1732584193
  MUT h1 AS Integer = 4023233417
  MUT h2 AS Integer = 2562383102
  MUT h3 AS Integer = 271733878
  MUT h4 AS Integer = 3285377520
  LET total AS Integer = len(msg)
  MUT base AS Integer = 0
  WHILE base < total
    LET w AS List OF Integer = __crypto_sha1Schedule(msg, base)
    MUT a AS Integer = h0
    MUT b AS Integer = h1
    MUT c AS Integer = h2
    MUT d AS Integer = h3
    MUT e AS Integer = h4
    MUT t AS Integer = 0
    WHILE t < 80
      LET t1 AS Integer = __crypto_add32(__crypto_rotl32(a, 5), __crypto_sha1F(t, b, c, d))
      LET t2 AS Integer = __crypto_add32(e, __crypto_sha1K(t))
      LET t3 AS Integer = __crypto_add32(t1, t2)
      LET temp AS Integer = __crypto_add32(t3, collections::get(w, t))
      e = d
      d = c
      c = __crypto_rotl32(b, 30)
      b = a
      a = temp
      t = t + 1
    END WHILE
    h0 = __crypto_add32(h0, a)
    h1 = __crypto_add32(h1, b)
    h2 = __crypto_add32(h2, c)
    h3 = __crypto_add32(h3, d)
    h4 = __crypto_add32(h4, e)
    base = base + 64
  END WHILE
  MUT digest AS List OF Byte = []
  digest = __crypto_appendBeWord(digest, h0)
  digest = __crypto_appendBeWord(digest, h1)
  digest = __crypto_appendBeWord(digest, h2)
  digest = __crypto_appendBeWord(digest, h3)
  digest = __crypto_appendBeWord(digest, h4)
  RETURN digest
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_sha1_bytes", BODY));
}
