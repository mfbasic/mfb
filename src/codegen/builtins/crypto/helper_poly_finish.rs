//! `__crypto_polyFinish` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Final reduction of the accumulator and addition of the `s` half of the key.
FUNC __crypto_polyFinish(ph0 AS Integer, ph1 AS Integer, ph2 AS Integer, ph3 AS Integer, ph4 AS Integer, key AS List OF Byte) AS List OF Byte
  MUT h0 AS Integer = ph0
  MUT h1 AS Integer = ph1
  MUT h2 AS Integer = ph2
  MUT h3 AS Integer = ph3
  MUT h4 AS Integer = ph4
  ' Fully carry h.
  MUT c AS Integer = bits::sr(h1, 26)
  h1 = bits::band(h1, 67108863)
  h2 = h2 + c
  c = bits::sr(h2, 26)
  h2 = bits::band(h2, 67108863)
  h3 = h3 + c
  c = bits::sr(h3, 26)
  h3 = bits::band(h3, 67108863)
  h4 = h4 + c
  c = bits::sr(h4, 26)
  h4 = bits::band(h4, 67108863)
  h0 = h0 + c * 5
  c = bits::sr(h0, 26)
  h0 = bits::band(h0, 67108863)
  h1 = h1 + c
  ' Compute h + -p (i.e. h - (2^130-5)) and select if h >= p.
  MUT g0 AS Integer = h0 + 5
  c = bits::sr(g0, 26)
  g0 = bits::band(g0, 67108863)
  MUT g1 AS Integer = h1 + c
  c = bits::sr(g1, 26)
  g1 = bits::band(g1, 67108863)
  MUT g2 AS Integer = h2 + c
  c = bits::sr(g2, 26)
  g2 = bits::band(g2, 67108863)
  MUT g3 AS Integer = h3 + c
  c = bits::sr(g3, 26)
  g3 = bits::band(g3, 67108863)
  MUT g4 AS Integer = h4 + c - 67108864
  ' mask = (g4 >> 63) ? all-ones-when-negative : select g. If g4 is negative
  ' (h < p), keep h; else keep g.
  IF g4 < 0 THEN
    ' keep h
    g0 = h0
    g1 = h1
    g2 = h2
    g3 = h3
    g4 = h4
  END IF
  ' Serialize h as four 32-bit little-endian words: pack the 26-bit limbs.
  LET f0 AS Integer = bits::band(bits::bor(g0, bits::sl(g1, 26)), 4294967295)
  LET f1 AS Integer = bits::band(bits::bor(bits::sr(g1, 6), bits::sl(g2, 20)), 4294967295)
  LET f2 AS Integer = bits::band(bits::bor(bits::sr(g2, 12), bits::sl(g3, 14)), 4294967295)
  LET f3 AS Integer = bits::band(bits::bor(bits::sr(g3, 18), bits::sl(g4, 8)), 4294967295)
  ' tag = (h + s) mod 2^128, s is key[16..32] as four LE words.
  LET s0 AS Integer = __crypto_leWord(key, 16)
  LET s1w AS Integer = __crypto_leWord(key, 20)
  LET s2w AS Integer = __crypto_leWord(key, 24)
  LET s3w AS Integer = __crypto_leWord(key, 28)
  MUT tag AS List OF Byte = []
  MUT carry AS Integer = 0
  LET a0 AS Integer = f0 + s0 + carry
  carry = bits::sr(a0, 32)
  tag = __crypto_appendLeWord(tag, bits::band(a0, 4294967295))
  LET a1 AS Integer = f1 + s1w + carry
  carry = bits::sr(a1, 32)
  tag = __crypto_appendLeWord(tag, bits::band(a1, 4294967295))
  LET a2 AS Integer = f2 + s2w + carry
  carry = bits::sr(a2, 32)
  tag = __crypto_appendLeWord(tag, bits::band(a2, 4294967295))
  LET a3 AS Integer = f3 + s3w + carry
  tag = __crypto_appendLeWord(tag, bits::band(a3, 4294967295))
  RETURN tag
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_polyFinish", BODY));
}
