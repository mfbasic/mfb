//! `__compress_deflateCore` — the raw DEFLATE (RFC 1951) encoder behind every `compress` encoder.
//!
//! One function owns all encoder state as locals, as the decoder core does (plan-137-B §4.1, whose
//! Phase 1 measurements chose one function over helpers called per symbol): the LSB-first bit
//! buffer, the output list, and the match finder's `head` / `prev` hash chains.
//!
//! - **Level 0** writes stored blocks of at most 65,535 bytes; empty input is one final empty
//!   stored block.
//! - **Levels 1–9** write fixed-Huffman blocks (`BTYPE = 01`), one per 64 KiB of input: a block
//!   ends at the first symbol that starts at or past 65,536 bytes after its first, so a match may
//!   run past the boundary. Empty input is one final block holding only end-of-block.
//! - **Matching is greedy** over hash chains of the next three bytes. Each position is looked up
//!   before it is inserted, so its own `prev` slot still links the position 32,768 bytes back and a
//!   match at the full RFC distance of 32,768 is found. A chain is followed for at most the level's
//!   `max_chain` candidates and stops at the first match of `nice_length` or longer. As zlib's
//!   `deflate_fast` does at levels 1–3, the positions inside a match are inserted only when the
//!   match is no longer than the level's `max_lazy` column (`max_insert_length`); levels 4–9 insert
//!   every position. The per-level numbers are zlib 1.2.12's `configuration_table`, fetched and
//!   pasted in plan-137-D §2.
//!
//! The output starts as `prefix` — a zlib or gzip header, or empty for raw DEFLATE — so a framing
//! helper never copies the compressed data to put a header in front of it. Written by in-place
//! `append` / same-size `set` on locals only (`.ai/collections.md`).
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

FUNC __compress_deflateCore(data AS List OF Byte, level AS Integer, prefix AS List OF Byte) AS List OF Byte
  LET n AS Integer = len(data)
  MUT out AS List OF Byte = prefix
  MUT pos AS Integer = 0
  MUT final AS Boolean = FALSE
  IF level = 0 THEN
    WHILE final = FALSE
      MUT chunk AS Integer = n - pos
      IF chunk > 65535 THEN
        chunk = 65535
      END IF
      final = pos + chunk >= n
      IF final THEN
        out = collections::append(out, toByte(1))
      ELSE
        out = collections::append(out, toByte(0))
      END IF
      out = collections::append(out, toByte(bits::band(chunk, 255)))
      out = collections::append(out, toByte(bits::sr(chunk, 8)))
      out = collections::append(out, toByte(bits::band(65535 - chunk, 255)))
      out = collections::append(out, toByte(bits::sr(65535 - chunk, 8)))
      MUT k AS Integer = 0
      WHILE k < chunk
        out = collections::append(out, collections::get(data, pos + k))
        k = k + 1
      END WHILE
      pos = pos + chunk
    END WHILE
    RETURN out
  END IF

  ' zlib 1.2.12 configuration_table columns: max_chain, nice_length, max_lazy.
  LET chains AS List OF Integer = [0, 4, 8, 32, 16, 32, 128, 256, 1024, 4096]
  LET nices AS List OF Integer = [0, 8, 16, 32, 16, 32, 128, 128, 258, 258]
  LET lazies AS List OF Integer = [0, 4, 5, 6, 4, 16, 16, 32, 128, 258]
  LET maxChain AS Integer = collections::get(chains, level)
  LET nice AS Integer = collections::get(nices, level)
  MUT maxInsert AS Integer = collections::get(lazies, level)
  IF level >= 4 THEN
    maxInsert = 258
  END IF

  ' --- match finder: head[hash] is the latest position with that hash, prev[p MOD 32768] the
  ' position before p with the same hash; -1 is none ---
  MUT head AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 32768
    head = collections::append(head, 0 - 1)
    i = i + 1
  END WHILE
  MUT prevSize AS Integer = n
  IF prevSize > 32768 THEN
    prevSize = 32768
  END IF
  MUT prev AS List OF Integer = []
  i = 0
  WHILE i < prevSize
    prev = collections::append(prev, 0 - 1)
    i = i + 1
  END WHILE
  ' --- bit writer: bitCount bits pending in bitBuf, least significant first ---
  MUT bitBuf AS Integer = 0
  MUT bitCount AS Integer = 0

  WHILE final = FALSE
    LET blockEnd AS Integer = pos + 65536
    final = blockEnd >= n
    ' BFINAL, then BTYPE = 01.
    IF final THEN
      bitBuf = bits::bor(bitBuf, bits::sl(3, bitCount))
    ELSE
      bitBuf = bits::bor(bitBuf, bits::sl(2, bitCount))
    END IF
    bitCount = bitCount + 3
    WHILE pos < blockEnd AND pos < n
      MUT bestLen AS Integer = 0
      MUT bestDist AS Integer = 0
      MUT maxLen AS Integer = n - pos
      IF maxLen > 258 THEN
        maxLen = 258
      END IF
      IF maxLen >= 3 THEN
        LET h AS Integer = bits::band(bits::bxor(bits::bxor(bits::sl(toInt(collections::get(data, pos)), 10), bits::sl(toInt(collections::get(data, pos + 1)), 5)), toInt(collections::get(data, pos + 2))), 32767)
        LET limit AS Integer = pos - 32768
        MUT cand AS Integer = collections::get(head, h)
        MUT chain AS Integer = maxChain
        bestLen = 2
        WHILE cand >= limit AND cand >= 0 AND chain > 0
          IF collections::get(data, cand + bestLen) = collections::get(data, pos + bestLen) THEN
            MUT k AS Integer = 0
            WHILE k < maxLen AND collections::get(data, cand + k) = collections::get(data, pos + k)
              k = k + 1
            END WHILE
            IF k > bestLen THEN
              bestLen = k
              bestDist = pos - cand
              IF k >= nice OR k >= maxLen THEN
                EXIT WHILE
              END IF
            END IF
          END IF
          cand = collections::get(prev, bits::band(cand, 32767))
          chain = chain - 1
        END WHILE
        prev = collections::set(prev, bits::band(pos, 32767), collections::get(head, h))
        head = collections::set(head, h, pos)
      END IF
      IF bestLen >= 3 THEN
        LET lengthCode AS Integer = collections::get(__COMPRESS_FIXED_LENGTH, bestLen)
        bitBuf = bits::bor(bitBuf, bits::sl(lengthCode / 32, bitCount))
        bitCount = bitCount + lengthCode MOD 32
        LET d1 AS Integer = bestDist - 1
        MUT dsym AS Integer = 0
        IF d1 < 256 THEN
          dsym = collections::get(__COMPRESS_DIST_CODE, d1)
        ELSE
          dsym = collections::get(__COMPRESS_DIST_CODE, 256 + bits::sr(d1, 7))
        END IF
        LET distValue AS Integer = collections::get(__COMPRESS_FIXED_DIST, dsym) + bits::sl(bestDist - collections::get(__COMPRESS_DIST_BASE, dsym), 5)
        bitBuf = bits::bor(bitBuf, bits::sl(distValue, bitCount))
        bitCount = bitCount + 5 + collections::get(__COMPRESS_DIST_EXTRA, dsym)
        IF bestLen <= maxInsert THEN
          MUT q AS Integer = pos + 1
          LET qEnd AS Integer = pos + bestLen
          WHILE q < qEnd
            IF q + 2 < n THEN
              LET hq AS Integer = bits::band(bits::bxor(bits::bxor(bits::sl(toInt(collections::get(data, q)), 10), bits::sl(toInt(collections::get(data, q + 1)), 5)), toInt(collections::get(data, q + 2))), 32767)
              prev = collections::set(prev, bits::band(q, 32767), collections::get(head, hq))
              head = collections::set(head, hq, q)
            END IF
            q = q + 1
          END WHILE
        END IF
        pos = pos + bestLen
      ELSE
        LET litCode AS Integer = collections::get(__COMPRESS_FIXED_LIT, toInt(collections::get(data, pos)))
        bitBuf = bits::bor(bitBuf, bits::sl(litCode / 16, bitCount))
        bitCount = bitCount + litCode MOD 16
        pos = pos + 1
      END IF
      WHILE bitCount >= 8
        out = collections::append(out, toByte(bits::band(bitBuf, 255)))
        bitBuf = bits::sr(bitBuf, 8)
        bitCount = bitCount - 8
      END WHILE
    END WHILE
    ' End of block: symbol 256 is the seven-bit fixed code 0000000.
    bitCount = bitCount + 7
    WHILE bitCount >= 8
      out = collections::append(out, toByte(bits::band(bitBuf, 255)))
      bitBuf = bits::sr(bitBuf, 8)
      bitCount = bitCount - 8
    END WHILE
  END WHILE
  IF bitCount > 0 THEN
    out = collections::append(out, toByte(bits::band(bitBuf, 255)))
  END IF
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_deflate_core",
        gate: HelperGate::WhenUsed(&["deflate", "zlibEncode", "gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
