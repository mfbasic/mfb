//! `__compress_codeLengths` / `__compress_canonicalCodes` — length-limited Huffman codes for the
//! encoder's dynamic blocks.
//!
//! Code lengths come from **package-merge** (Larmore and Hirschberg, "A fast algorithm for optimal
//! length-limited Huffman codes", JACM 37(3):464–473, 1990, doi:10.1145/79147.79150), which is exact
//! under the limit: 15 bits for the literal/length and distance codes, 7 for the code-length code.
//! The used symbols are sorted by `(weight, symbol)` and form the first list. `limit - 1` times,
//! adjacent pairs of the current list are packaged and merged with the leaves, a leaf going first on
//! equal weight. The first `2n - 2` items of the last list are expanded, and a symbol's code length
//! is the number of times its leaf appears. Items live in a node pool (weight, and the leaf's symbol
//! or the package's two children), so no item carries a copy of its leaves.
//!
//! A code with fewer than two used symbols is padded as zlib 1.2.12's `build_tree` pads it
//! (`trees.c` 641–652, "The pkzip format requires that at least one distance code exists"): symbol
//! `max_code + 1` while `max_code < 2`, else symbol 0, each given weight 1, so every emitted code is
//! complete (plan-137-E §2). The padded symbol is never emitted, and the block's cost uses the real
//! frequencies.
//!
//! `__compress_canonicalCodes` assigns RFC 1951 §3.2.2 canonical codes and returns each, bit-reversed
//! for the least-significant-bit-first writer, as `reversed code * 16 + length`.
//!
//! Correctness is judged by zlib decoding the encoder's output and by the oracle's per-block audit
//! (`tools/oracles/compress`, `encode-*` modes, which include Fibonacci frequencies that force the
//! 15-bit limit), and by this package's own decoder, which refuses any incomplete or over-subscribed
//! code.
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

' A fresh list of `count` zeros.
FUNC __compress_zeroList(count AS Integer) AS List OF Integer
  MUT t AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < count
    t = collections::append(t, 0)
    i = i + 1
  END WHILE
  RETURN t
END FUNC

' Code lengths of at most `limit` bits for `freqs` (one entry per symbol, 0 = unused): package-merge.
FUNC __compress_codeLengths(freqs AS List OF Integer, limit AS Integer) AS List OF Integer
  LET count AS Integer = len(freqs)
  MUT f AS List OF Integer = freqs
  MUT used AS Integer = 0
  MUT maxCode AS Integer = 0 - 1
  MUT s AS Integer = 0
  WHILE s < count
    IF collections::get(f, s) > 0 THEN
      used = used + 1
      maxCode = s
    END IF
    s = s + 1
  END WHILE
  ' zlib's build_tree: at least two codes of non-zero frequency.
  WHILE used < 2
    MUT node AS Integer = 0
    IF maxCode < 2 THEN
      maxCode = maxCode + 1
      node = maxCode
    END IF
    f = collections::set(f, node, 1)
    used = used + 1
  END WHILE

  ' The leaves, sorted by (weight, symbol): insertion sort of weight * 1024 + symbol.
  MUT keys AS List OF Integer = []
  s = 0
  WHILE s < count
    LET w AS Integer = collections::get(f, s)
    IF w > 0 THEN
      LET key AS Integer = w * 1024 + s
      keys = collections::append(keys, key)
      MUT j AS Integer = len(keys) - 1
      WHILE j > 0 AND collections::get(keys, j - 1) > key
        keys = collections::set(keys, j, collections::get(keys, j - 1))
        j = j - 1
      END WHILE
      keys = collections::set(keys, j, key)
    END IF
    s = s + 1
  END WHILE
  LET n AS Integer = len(keys)

  ' Node pool: weight; a leaf's symbol or a package's first child; -1 or a package's second child.
  MUT poolW AS List OF Integer = []
  MUT poolA AS List OF Integer = []
  MUT poolB AS List OF Integer = []
  MUT cur AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < n
    LET key AS Integer = collections::get(keys, i)
    poolW = collections::append(poolW, key / 1024)
    poolA = collections::append(poolA, key MOD 1024)
    poolB = collections::append(poolB, 0 - 1)
    cur = collections::append(cur, i)
    i = i + 1
  END WHILE

  MUT packLevel AS Integer = 1
  WHILE packLevel < limit
    MUT packages AS List OF Integer = []
    MUT p AS Integer = 0
    WHILE p + 1 < len(cur)
      LET a AS Integer = collections::get(cur, p)
      LET b AS Integer = collections::get(cur, p + 1)
      poolW = collections::append(poolW, collections::get(poolW, a) + collections::get(poolW, b))
      poolA = collections::append(poolA, a)
      poolB = collections::append(poolB, b)
      packages = collections::append(packages, len(poolW) - 1)
      p = p + 2
    END WHILE
    MUT merged AS List OF Integer = []
    MUT li AS Integer = 0
    MUT pi AS Integer = 0
    WHILE li < n OR pi < len(packages)
      IF pi >= len(packages) THEN
        merged = collections::append(merged, li)
        li = li + 1
      ELSEIF li >= n THEN
        merged = collections::append(merged, collections::get(packages, pi))
        pi = pi + 1
      ELSEIF collections::get(poolW, li) <= collections::get(poolW, collections::get(packages, pi)) THEN
        merged = collections::append(merged, li)
        li = li + 1
      ELSE
        merged = collections::append(merged, collections::get(packages, pi))
        pi = pi + 1
      END IF
    END WHILE
    cur = merged
    packLevel = packLevel + 1
  END WHILE

  ' Expand the first 2n - 2 items; each leaf occurrence adds one bit to its symbol's length.
  MUT lens AS List OF Integer = __compress_zeroList(count)
  MUT stack AS List OF Integer = []
  MUT top AS Integer = 0
  i = 0
  WHILE i < 2 * n - 2
    stack = collections::append(stack, collections::get(cur, i))
    top = 1
    WHILE top > 0
      top = top - 1
      LET item AS Integer = collections::get(stack, top)
      stack = collections::removeAt(stack, top)
      IF collections::get(poolB, item) < 0 THEN
        LET sym AS Integer = collections::get(poolA, item)
        lens = collections::set(lens, sym, collections::get(lens, sym) + 1)
      ELSE
        stack = collections::append(stack, collections::get(poolA, item))
        stack = collections::append(stack, collections::get(poolB, item))
        top = top + 2
      END IF
    END WHILE
    i = i + 1
  END WHILE
  RETURN lens
END FUNC

' RFC 1951 3.2.2 canonical codes for `lens`, as `reversed code * 16 + length` (0 for an unused symbol).
FUNC __compress_canonicalCodes(lens AS List OF Integer) AS List OF Integer
  LET count AS Integer = len(lens)
  MUT blCount AS List OF Integer = __compress_zeroList(16)
  MUT s AS Integer = 0
  WHILE s < count
    LET l AS Integer = collections::get(lens, s)
    IF l > 0 THEN
      blCount = collections::set(blCount, l, collections::get(blCount, l) + 1)
    END IF
    s = s + 1
  END WHILE
  MUT nextCode AS List OF Integer = __compress_zeroList(16)
  MUT code AS Integer = 0
  MUT b AS Integer = 1
  WHILE b <= 15
    code = bits::sl(code + collections::get(blCount, b - 1), 1)
    nextCode = collections::set(nextCode, b, code)
    b = b + 1
  END WHILE
  MUT t AS List OF Integer = []
  s = 0
  WHILE s < count
    LET l AS Integer = collections::get(lens, s)
    IF l > 0 THEN
      LET c AS Integer = collections::get(nextCode, l)
      nextCode = collections::set(nextCode, l, c + 1)
      t = collections::append(t, __compress_reverseBits(c, l) * 16 + l)
    ELSE
      t = collections::append(t, 0)
    END IF
    s = s + 1
  END WHILE
  RETURN t
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_package_merge",
        gate: HelperGate::WhenUsed(&["deflate", "zlibEncode", "gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
