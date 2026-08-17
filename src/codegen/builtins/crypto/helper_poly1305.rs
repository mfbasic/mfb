//! `__crypto_poly1305` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Poly1305 MAC of `msg` under one-time `key` (32 bytes): 16-byte tag.
FUNC __crypto_poly1305(key AS List OF Byte, msg AS List OF Byte) AS List OF Byte
  LET r AS List OF Integer = __crypto_polyR(key)
  LET r0 AS Integer = collections::get(r, 0)
  LET r1 AS Integer = collections::get(r, 1)
  LET r2 AS Integer = collections::get(r, 2)
  LET r3 AS Integer = collections::get(r, 3)
  LET r4 AS Integer = collections::get(r, 4)
  LET s1 AS Integer = r1 * 5
  LET s2 AS Integer = r2 * 5
  LET s3 AS Integer = r3 * 5
  LET s4 AS Integer = r4 * 5
  MUT h0 AS Integer = 0
  MUT h1 AS Integer = 0
  MUT h2 AS Integer = 0
  MUT h3 AS Integer = 0
  MUT h4 AS Integer = 0
  LET n AS Integer = len(msg)
  MUT offset AS Integer = 0
  WHILE offset < n
    LET remaining AS Integer = n - offset
    MUT take AS Integer = 16
    IF remaining < 16 THEN
      take = remaining
    END IF
    ' Assemble the (up to) 16-byte block with the 1-bit high terminator.
    MUT block AS List OF Byte = []
    MUT bi AS Integer = 0
    WHILE bi < take
      block = collections::append(block, collections::get(msg, offset + bi))
      bi = bi + 1
    END WHILE
    MUT hibit AS Integer = 16777216
    IF take < 16 THEN
      block = collections::append(block, toByte(1))
      WHILE len(block) < 16
        block = collections::append(block, toByte(0))
      END WHILE
      hibit = 0
    END IF
    LET t0 AS Integer = __crypto_leWord(block, 0)
    LET t1 AS Integer = __crypto_leWord(block, 4)
    LET t2 AS Integer = __crypto_leWord(block, 8)
    LET t3 AS Integer = __crypto_leWord(block, 12)
    h0 = h0 + bits::band(t0, 67108863)
    h1 = h1 + bits::band(bits::bor(bits::sr(t0, 26), bits::sl(t1, 6)), 67108863)
    h2 = h2 + bits::band(bits::bor(bits::sr(t1, 20), bits::sl(t2, 12)), 67108863)
    h3 = h3 + bits::band(bits::bor(bits::sr(t2, 14), bits::sl(t3, 18)), 67108863)
    h4 = h4 + bits::bor(bits::sr(t3, 8), hibit)
    ' d = h * r  (schoolbook, with s_i = 5*r_i for the reduction wrap).
    LET d0 AS Integer = h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1
    LET d1 AS Integer = h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2
    LET d2 AS Integer = h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3
    LET d3 AS Integer = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4
    LET d4 AS Integer = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0
    ' Carry-propagate the five limbs.
    MUT c AS Integer = bits::sr(d0, 26)
    h0 = bits::band(d0, 67108863)
    LET e1 AS Integer = d1 + c
    c = bits::sr(e1, 26)
    h1 = bits::band(e1, 67108863)
    LET e2 AS Integer = d2 + c
    c = bits::sr(e2, 26)
    h2 = bits::band(e2, 67108863)
    LET e3 AS Integer = d3 + c
    c = bits::sr(e3, 26)
    h3 = bits::band(e3, 67108863)
    LET e4 AS Integer = d4 + c
    c = bits::sr(e4, 26)
    h4 = bits::band(e4, 67108863)
    h0 = h0 + c * 5
    c = bits::sr(h0, 26)
    h0 = bits::band(h0, 67108863)
    h1 = h1 + c
    offset = offset + take
  END WHILE
  RETURN __crypto_polyFinish(h0, h1, h2, h3, h4, key)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_poly1305", BODY));
}
