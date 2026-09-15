//! `__compress_buildTable` — canonical Huffman decode tables for `compress`'s inflate.
//!
//! RFC 1951 §3.2.2 canonical codes, validated as zlib 1.2.12 `inftrees.c` `inflate_table` does:
//! an over-subscribed set is refused; an incomplete set is refused unless it is a literal/length
//! or distance set whose longest code is 1 bit; an all-zero set yields a table of unused slots,
//! which fails only if a symbol is decoded from it (plan-137-B §4.2, confirmed by
//! `tools/oracles/compress/probe.sh`). Two levels: a primary table indexed by the next `root`
//! bits, entries `symbol * 16 + length`; codes longer than `root` get a sub-table addressed by a
//! pointer entry `1048576 + offset * 16 + subBits`; unused slots are -1. Also the fixed
//! literal/length code lengths (RFC 1951 §3.2.6) and a list-filling helper.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on the decoders. A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

FUNC __compress_filled(count AS Integer, value AS Integer) AS List OF Integer
  MUT t AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < count
    t = collections::append(t, value)
    i = i + 1
  END WHILE
  RETURN t
END FUNC

FUNC __compress_buildTable(lens AS List OF Integer, start AS Integer, count AS Integer, root AS Integer, kind AS Integer) AS List OF Integer
  MUT bl AS List OF Integer = __compress_filled(16, 0)
  MUT i AS Integer = 0
  WHILE i < count
    LET l AS Integer = collections::get(lens, start + i)
    bl = collections::set(bl, l, collections::get(bl, l) + 1)
    i = i + 1
  END WHILE
  MUT maxLen AS Integer = 15
  WHILE maxLen >= 1 AND collections::get(bl, maxLen) = 0
    maxLen = maxLen - 1
  END WHILE
  LET size AS Integer = bits::sl(1, root)
  MUT table AS List OF Integer = __compress_filled(size, -1)
  IF maxLen = 0 THEN
    RETURN table
  END IF
  MUT left AS Integer = 1
  MUT bitLen AS Integer = 1
  WHILE bitLen <= 15
    left = left * 2 - collections::get(bl, bitLen)
    IF left < 0 THEN
      FAIL error(77050003, "compress: over-subscribed code set")
    END IF
    bitLen = bitLen + 1
  END WHILE
  IF left > 0 AND (kind = 0 OR maxLen <> 1) THEN
    FAIL error(77050003, "compress: incomplete code set")
  END IF
  MUT nextCode AS List OF Integer = __compress_filled(16, 0)
  MUT code AS Integer = 0
  bitLen = 1
  WHILE bitLen <= 15
    code = (code + collections::get(bl, bitLen - 1)) * 2
    nextCode = collections::set(nextCode, bitLen, code)
    bitLen = bitLen + 1
  END WHILE
  LET subBits AS Integer = maxLen - root
  MUT sym AS Integer = 0
  WHILE sym < count
    LET l AS Integer = collections::get(lens, start + sym)
    IF l > 0 THEN
      LET c AS Integer = collections::get(nextCode, l)
      nextCode = collections::set(nextCode, l, c + 1)
      MUT rev AS Integer = 0
      MUT b AS Integer = 0
      WHILE b < l
        rev = rev * 2 + (c / bits::sl(1, b)) MOD 2
        b = b + 1
      END WHILE
      IF l <= root THEN
        LET directStep AS Integer = bits::sl(1, l)
        MUT dj AS Integer = rev
        WHILE dj < size
          table = collections::set(table, dj, sym * 16 + l)
          dj = dj + directStep
        END WHILE
      ELSE
        LET prefix AS Integer = rev MOD size
        MUT e AS Integer = collections::get(table, prefix)
        MUT off AS Integer = 0
        IF e < 0 THEN
          off = len(table)
          MUT k AS Integer = 0
          LET newSubSize AS Integer = bits::sl(1, subBits)
          WHILE k < newSubSize
            table = collections::append(table, -1)
            k = k + 1
          END WHILE
          table = collections::set(table, prefix, 1048576 + off * 16 + subBits)
        ELSE
          off = (e - 1048576) / 16
        END IF
        LET sl2 AS Integer = l - root
        LET subStep AS Integer = bits::sl(1, sl2)
        LET subSize AS Integer = bits::sl(1, subBits)
        MUT sj AS Integer = rev / size
        WHILE sj < subSize
          table = collections::set(table, off + sj, sym * 16 + sl2)
          sj = sj + subStep
        END WHILE
      END IF
    END IF
    sym = sym + 1
  END WHILE
  RETURN table
END FUNC

FUNC __compress_fixedLitLens() AS List OF Integer
  MUT t AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 288
    MUT l AS Integer = 8
    IF i >= 144 AND i < 256 THEN
      l = 9
    ELSEIF i >= 256 AND i < 280 THEN
      l = 7
    END IF
    t = collections::append(t, l)
    i = i + 1
  END WHILE
  RETURN t
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_huffman_table",
        gate: HelperGate::WhenUsed(&["inflate", "zlibDecode", "gzipDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
