//! `__COMPRESS_FIXED_LIT` / `__COMPRESS_FIXED_LENGTH` / `__COMPRESS_FIXED_DIST` /
//! `__COMPRESS_DIST_CODE` — the encoder's RFC 1951 §3.2.6 fixed Huffman codes, bit-reversed.
//!
//! RFC 1951 §3.1.1 packs Huffman codes most-significant bit first into a stream that is
//! otherwise least-significant bit first, so every code is stored reversed and the encoder's bit
//! writer ORs it in unchanged. Built once at program start by functions rather than written as
//! literals (a list literal lowers to per-element initializer code; plan-137-A Corrections). The
//! builders call `__compress_lengthTable` / `__compress_distanceTable` directly instead of reading
//! the decoder tables' globals, so no global initializer depends on another file's. The fixed code
//! boundaries are pinned against the RFC by `tests::fixed_code_builder_matches_rfc1951` in
//! `compress/mod.rs`; the codes themselves are judged by zlib decoding the encoder's output
//! (`tools/oracles/compress`, `encode-*` modes).
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on the encoders. A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
pub(super) const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' `code`'s low `length` bits in reverse order.
FUNC __compress_reverseBits(code AS Integer, length AS Integer) AS Integer
  MUT r AS Integer = 0
  MUT c AS Integer = code
  MUT i AS Integer = 0
  WHILE i < length
    r = bits::bor(bits::sl(r, 1), bits::band(c, 1))
    c = bits::sr(c, 1)
    i = i + 1
  END WHILE
  RETURN r
END FUNC

' RFC 1951 3.2.6 fixed literal/length code of symbols 0..287, as `reversed code * 16 + length`.
FUNC __compress_fixedLitCodes() AS List OF Integer
  MUT t AS List OF Integer = []
  MUT sym AS Integer = 0
  WHILE sym < 288
    MUT code AS Integer = 0
    MUT length AS Integer = 0
    IF sym < 144 THEN
      code = 48 + sym
      length = 8
    ELSEIF sym < 256 THEN
      code = 400 + sym - 144
      length = 9
    ELSEIF sym < 280 THEN
      code = sym - 256
      length = 7
    ELSE
      code = 192 + sym - 280
      length = 8
    END IF
    t = collections::append(t, __compress_reverseBits(code, length) * 16 + length)
    sym = sym + 1
  END WHILE
  RETURN t
END FUNC

' Every match length 0..258 as its fixed length code followed by its extra bits, as
' `bits * 32 + bit count`. Lengths 0..2 are never emitted and hold 0; 258 is code 285.
FUNC __compress_fixedLengthCodes() AS List OF Integer
  LET lit AS List OF Integer = __compress_fixedLitCodes()
  LET base AS List OF Integer = __compress_lengthTable(TRUE)
  LET extra AS List OF Integer = __compress_lengthTable(FALSE)
  MUT t AS List OF Integer = [0, 0, 0]
  MUT li AS Integer = 0
  MUT length AS Integer = 3
  WHILE length <= 258
    WHILE li < 28 AND length >= collections::get(base, li + 1)
      li = li + 1
    END WHILE
    LET entry AS Integer = collections::get(lit, 257 + li)
    LET codeBits AS Integer = entry MOD 16
    LET value AS Integer = entry / 16 + bits::sl(length - collections::get(base, li), codeBits)
    t = collections::append(t, value * 32 + codeBits + collections::get(extra, li))
    length = length + 1
  END WHILE
  RETURN t
END FUNC

' RFC 1951 3.2.6 fixed distance codes 0..29: five bits each, reversed.
FUNC __compress_fixedDistanceCodes() AS List OF Integer
  MUT t AS List OF Integer = []
  MUT sym AS Integer = 0
  WHILE sym < 30
    t = collections::append(t, __compress_reverseBits(sym, 5))
    sym = sym + 1
  END WHILE
  RETURN t
END FUNC

' The distance code of every `distance - 1`, indexed as zlib's `dist_code`: values 0..255
' directly, then `256 + (value >> 7)` for 256..32767 -- every code from 16 up starts on a
' multiple of 128, so one entry per 128 values is exact.
FUNC __compress_distanceCodes() AS List OF Integer
  LET base AS List OF Integer = __compress_distanceTable(TRUE)
  MUT t AS List OF Integer = []
  MUT code AS Integer = 0
  MUT v AS Integer = 0
  WHILE v < 256
    WHILE code < 29 AND collections::get(base, code + 1) - 1 <= v
      code = code + 1
    END WHILE
    t = collections::append(t, code)
    v = v + 1
  END WHILE
  code = 0
  MUT j AS Integer = 0
  WHILE j < 256
    WHILE code < 29 AND collections::get(base, code + 1) - 1 <= j * 128
      code = code + 1
    END WHILE
    t = collections::append(t, code)
    j = j + 1
  END WHILE
  RETURN t
END FUNC

LET __COMPRESS_FIXED_LIT AS List OF Integer = __compress_fixedLitCodes()
LET __COMPRESS_FIXED_LENGTH AS List OF Integer = __compress_fixedLengthCodes()
LET __COMPRESS_FIXED_DIST AS List OF Integer = __compress_fixedDistanceCodes()
LET __COMPRESS_DIST_CODE AS List OF Integer = __compress_distanceCodes()"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_deflate_codes",
        gate: HelperGate::WhenUsed(&["deflate", "zlibEncode", "gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
