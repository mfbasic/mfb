//! `__compress_treeRle` / `__compress_dynamicHeader` — the header of a dynamic-Huffman block
//! (RFC 1951 §3.2.7).
//!
//! `__compress_treeRle` run-length encodes one tree's code lengths into code-length symbols as zlib
//! 1.2.12's `send_tree` does (`trees.c` 748–792, fetched for plan-137-E): a length repeated 3–6 times
//! after its first appearance becomes symbol 16, a run of 3–10 zeros symbol 17, a run of 11–138 zeros
//! symbol 18, with zlib's `max_count` / `min_count` state machine and its guard past the last code.
//! The literal/length and distance trees are encoded separately, as zlib does, so no repeat crosses
//! from one tree into the other. Each entry is `symbol + extra bits value * 32`.
//!
//! `__compress_dynamicHeader` returns the whole header as `value * 32 + bit count` entries, which
//! the core both writes and sums for the block's cost. `HLIT` is the last used literal/length code
//! plus one, at least 257; `HDIST` is the last used distance code plus one, at least 1. The code-length
//! code is built with a 7-bit limit, and `HCLEN` drops trailing unused entries of the §3.2.7 order,
//! keeping at least four, as zlib's `build_bl_tree` does (`trees.c` 820–822).
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on the encoders. A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' zlib 1.2.12 send_tree over lens[0..codes): code-length symbols as `symbol + extra * 32`.
FUNC __compress_treeRle(lens AS List OF Integer, codes AS Integer) AS List OF Integer
  MUT out AS List OF Integer = []
  MUT prevlen AS Integer = 0 - 1
  MUT nextlen AS Integer = collections::get(lens, 0)
  MUT count AS Integer = 0
  MUT maxCount AS Integer = 7
  MUT minCount AS Integer = 4
  IF nextlen = 0 THEN
    maxCount = 138
    minCount = 3
  END IF
  MUT n AS Integer = 0
  WHILE n < codes
    LET curlen AS Integer = nextlen
    IF n + 1 < codes THEN
      nextlen = collections::get(lens, n + 1)
    ELSE
      nextlen = 0 - 1
    END IF
    count = count + 1
    IF count < maxCount AND curlen = nextlen THEN
      n = n + 1
      CONTINUE WHILE
    END IF
    IF count < minCount THEN
      WHILE count > 0
        out = collections::append(out, curlen)
        count = count - 1
      END WHILE
    ELSEIF curlen <> 0 THEN
      IF curlen <> prevlen THEN
        out = collections::append(out, curlen)
        count = count - 1
      END IF
      out = collections::append(out, 16 + (count - 3) * 32)
    ELSEIF count <= 10 THEN
      out = collections::append(out, 17 + (count - 3) * 32)
    ELSE
      out = collections::append(out, 18 + (count - 11) * 32)
    END IF
    count = 0
    prevlen = curlen
    IF nextlen = 0 THEN
      maxCount = 138
      minCount = 3
    ELSEIF curlen = nextlen THEN
      maxCount = 6
      minCount = 3
    ELSE
      maxCount = 7
      minCount = 4
    END IF
    n = n + 1
  END WHILE
  RETURN out
END FUNC

' The dynamic block header after BFINAL and BTYPE, as `value * 32 + bit count` entries.
FUNC __compress_dynamicHeader(litLens AS List OF Integer, distLens AS List OF Integer) AS List OF Integer
  MUT hlit AS Integer = 286
  WHILE hlit > 257 AND collections::get(litLens, hlit - 1) = 0
    hlit = hlit - 1
  END WHILE
  MUT hdist AS Integer = 30
  WHILE hdist > 1 AND collections::get(distLens, hdist - 1) = 0
    hdist = hdist - 1
  END WHILE
  LET litRle AS List OF Integer = __compress_treeRle(litLens, hlit)
  LET distRle AS List OF Integer = __compress_treeRle(distLens, hdist)
  MUT clFreq AS List OF Integer = __compress_zeroList(19)
  MUT i AS Integer = 0
  WHILE i < len(litRle)
    LET sym AS Integer = collections::get(litRle, i) MOD 32
    clFreq = collections::set(clFreq, sym, collections::get(clFreq, sym) + 1)
    i = i + 1
  END WHILE
  i = 0
  WHILE i < len(distRle)
    LET sym AS Integer = collections::get(distRle, i) MOD 32
    clFreq = collections::set(clFreq, sym, collections::get(clFreq, sym) + 1)
    i = i + 1
  END WHILE
  LET clLens AS List OF Integer = __compress_codeLengths(clFreq, 7)
  LET clCodes AS List OF Integer = __compress_canonicalCodes(clLens)
  MUT hclen AS Integer = 19
  WHILE hclen > 4 AND collections::get(clLens, collections::get(__COMPRESS_CL_ORDER, hclen - 1)) = 0
    hclen = hclen - 1
  END WHILE

  MUT out AS List OF Integer = []
  out = collections::append(out, (hlit - 257) * 32 + 5)
  out = collections::append(out, (hdist - 1) * 32 + 5)
  out = collections::append(out, (hclen - 4) * 32 + 4)
  i = 0
  WHILE i < hclen
    out = collections::append(out, collections::get(clLens, collections::get(__COMPRESS_CL_ORDER, i)) * 32 + 3)
    i = i + 1
  END WHILE
  MUT part AS Integer = 0
  WHILE part < 2
    MUT rle AS List OF Integer = litRle
    IF part = 1 THEN
      rle = distRle
    END IF
    i = 0
    WHILE i < len(rle)
      LET entry AS Integer = collections::get(rle, i)
      LET sym AS Integer = entry MOD 32
      LET code AS Integer = collections::get(clCodes, sym)
      out = collections::append(out, (code / 16) * 32 + code MOD 16)
      IF sym = 16 THEN
        out = collections::append(out, (entry / 32) * 32 + 2)
      ELSEIF sym = 17 THEN
        out = collections::append(out, (entry / 32) * 32 + 3)
      ELSEIF sym = 18 THEN
        out = collections::append(out, (entry / 32) * 32 + 7)
      END IF
      i = i + 1
    END WHILE
    part = part + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_dynamic_block",
        gate: HelperGate::WhenUsed(&["deflate", "zlibEncode", "gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
