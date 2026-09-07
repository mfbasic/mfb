//! `__crypto_blake2bCompress` — shared private helper for the `crypto` package.
//!
//! The BLAKE2b compression function F (RFC 7693 §3.2): twelve rounds of eight G
//! steps over one 128-byte block, driven by the SIGMA schedule.
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

' The BLAKE2b compression function F (RFC 7693 section 3.2): mix the 128-byte block
' at `off` into the chain `h`, with `t` bytes counted so far and `last` set on the
' final block. The high half of the 128-bit counter is always zero here.
FUNC __crypto_blake2bCompress(h AS List OF Integer, blk AS List OF Byte, t AS Integer, last AS Boolean) AS List OF Integer
  MUT m AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    m = collections::append(m, __crypto_leLane(blk, i * 8))
    i = i + 1
  END WHILE
  MUT v AS List OF Integer = []
  i = 0
  WHILE i < 8
    v = collections::append(v, collections::get(h, i))
    i = i + 1
  END WHILE
  i = 0
  WHILE i < 8
    v = collections::append(v, collections::get(__CRYPTO_IV512, i))
    i = i + 1
  END WHILE
  v = collections::set(v, 12, bits::bxor(collections::get(v, 12), t))
  IF last THEN
    v = collections::set(v, 14, bits::bnot(collections::get(v, 14)))
  END IF
  MUT r AS Integer = 0
  WHILE r < 12
    LET s AS Integer = r * 16
    v = __crypto_blake2bG(v, 0, 4, 8, 12, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 1))))
    v = __crypto_blake2bG(v, 1, 5, 9, 13, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 2))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 3))))
    v = __crypto_blake2bG(v, 2, 6, 10, 14, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 4))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 5))))
    v = __crypto_blake2bG(v, 3, 7, 11, 15, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 6))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 7))))
    v = __crypto_blake2bG(v, 0, 5, 10, 15, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 8))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 9))))
    v = __crypto_blake2bG(v, 1, 6, 11, 12, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 10))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 11))))
    v = __crypto_blake2bG(v, 2, 7, 8, 13, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 12))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 13))))
    v = __crypto_blake2bG(v, 3, 4, 9, 14, collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 14))), collections::get(m, toInt(collections::get(__CRYPTO_B2B_SIGMA, s + 15))))
    r = r + 1
  END WHILE
  MUT out AS List OF Integer = []
  i = 0
  WHILE i < 8
    out = collections::append(out, bits::bxor(collections::get(h, i), bits::bxor(collections::get(v, i), collections::get(v, i + 8))))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_blake2bCompress",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
