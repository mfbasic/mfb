//! `__canvas_inflate` — DEFLATE (RFC 1951) decompression, in MFBASIC.
//!
//! Written here rather than pulled in, for the reason the whole letter is hand-rolled
//! (plan-98-G Correction 1): `grep -rn "inflate\|deflate" src/codegen/builtins/`
//! returned nothing before this file, and a PNG cannot be read without it.
//!
//! It is a straight reference implementation — no window-copy tricks, no table-driven
//! decoding — because the property that matters is that it produces the same bytes on
//! every target. Speed is bounded by what it is used for: an image is decoded once, at
//! load, and never again.
//!
//! **The output is built in a local and written back once.** `collections::append` is
//! in-place only for a local of the function doing the write, so appending straight into
//! a global or a parameter copies the whole buffer per byte (see `.ai/collections.md`).
//! The back-reference copy reads from that same local, which is what makes an LZ77 match
//! that overlaps its own output work by construction: it reads bytes it has just written.
//!
//! Registered via `add_helper`; body byte-significant (2-space indent → `.ncode`
//! columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The bit reader. DEFLATE is LSB-first *within* a byte, and Huffman codes are packed
/// MSB-first *within* a code — the two orders that trip every hand-written inflate.
///
/// The reader is a pair of plain integers (byte position, bit position) threaded
/// through as a two-element list rather than held in globals, so two decompressions can
/// never interleave through shared state. `__canvas_bitsAt` returns the value and the
/// caller advances the cursor itself, which keeps the reader free of side effects.
#[rustfmt::skip]
const INFLATE_BITS: &str =
r#"FUNC __canvas_bitAt(data AS List OF Byte, pos AS Integer) AS Integer
  LET byte AS Integer = toInt(collections::getOr(data, pos / 8, toByte(0)))
  RETURN (byte / __canvas_pow2(pos MOD 8)) MOD 2
END FUNC

FUNC __canvas_bitsAt(data AS List OF Byte, pos AS Integer, count AS Integer) AS Integer
  MUT value AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < count
    value = value + __canvas_bitAt(data, pos + i) * __canvas_pow2(i)
    i = i + 1
  END WHILE
  RETURN value
END FUNC

FUNC __canvas_pow2(n AS Integer) AS Integer
  MUT v AS Integer = 1
  MUT i AS Integer = 0
  WHILE i < n
    v = v * 2
    i = i + 1
  END WHILE
  RETURN v
END FUNC"#;

/// Canonical Huffman decoding from a code-length list, per RFC 1951 §3.2.2.
///
/// The tree is never built. A canonical code is fully described by its length list, so
/// decoding walks lengths 1..15 keeping a running first-code and count — which is both
/// shorter than a tree and free of the allocation a tree would cost per block.
///
/// `__canvas_huffDecode` returns `-1` for a bit pattern no code matches. Every caller
/// treats that as a malformed stream rather than as a symbol, because the alternative —
/// carrying on with a wrong symbol — turns a corrupt file into a plausible wrong image.
#[rustfmt::skip]
const INFLATE_HUFF: &str =
r#"FUNC __canvas_huffCounts(lengths AS List OF Integer) AS List OF Integer
  MUT counts AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 16
    counts = collections::append(counts, 0)
    i = i + 1
  END WHILE
  FOR EACH l IN lengths
    IF l > 0 AND l < 16 THEN
      counts = collections::set(counts, l, collections::getOr(counts, l, 0) + 1)
    END IF
  NEXT
  RETURN counts
END FUNC

FUNC __canvas_huffOffsets(counts AS List OF Integer) AS List OF Integer
  MUT offsets AS List OF Integer = []
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 16
    offsets = collections::append(offsets, total)
    total = total + collections::getOr(counts, i, 0)
    i = i + 1
  END WHILE
  RETURN offsets
END FUNC

FUNC __canvas_huffSymbols(lengths AS List OF Integer, counts AS List OF Integer) AS List OF Integer
  MUT offsets AS List OF Integer = __canvas_huffOffsets(counts)
  MUT symbols AS List OF Integer = []
  MUT i AS Integer = 0
  LET n AS Integer = len(lengths)
  WHILE i < n
    symbols = collections::append(symbols, 0)
    i = i + 1
  END WHILE
  i = 0
  WHILE i < n
    LET l AS Integer = collections::getOr(lengths, i, 0)
    IF l > 0 AND l < 16 THEN
      LET at AS Integer = collections::getOr(offsets, l, 0)
      symbols = collections::set(symbols, at, i)
      offsets = collections::set(offsets, l, at + 1)
    END IF
    i = i + 1
  END WHILE
  RETURN symbols
END FUNC

FUNC __canvas_huffDecode(data AS List OF Byte, pos AS Integer, counts AS List OF Integer, symbols AS List OF Integer) AS List OF Integer
  MUT code AS Integer = 0
  MUT first AS Integer = 0
  MUT index AS Integer = 0
  MUT length AS Integer = 1
  WHILE length < 16
    code = code * 2 + __canvas_bitAt(data, pos + length - 1)
    LET count AS Integer = collections::getOr(counts, length, 0)
    IF code - first < count THEN
      RETURN [collections::getOr(symbols, index + code - first, 0), length]
    END IF
    index = index + count
    first = (first + count) * 2
    length = length + 1
  END WHILE
  RETURN [0 - 1, 15]
END FUNC"#;

/// The fixed code lengths of RFC 1951 §3.2.6, built rather than tabulated.
///
/// Spelling the 288 literal lengths out would be 288 lines of source in a package whose
/// every byte is rendered into the assembled builtin source; the four ranges are the
/// specification's own description of them, so building them is also the more faithful
/// transcription.
#[rustfmt::skip]
const INFLATE_FIXED: &str =
r#"FUNC __canvas_fixedLitLengths() AS List OF Integer
  MUT lengths AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 288
    IF i < 144 THEN
      lengths = collections::append(lengths, 8)
    ELSEIF i < 256 THEN
      lengths = collections::append(lengths, 9)
    ELSEIF i < 280 THEN
      lengths = collections::append(lengths, 7)
    ELSE
      lengths = collections::append(lengths, 8)
    END IF
    i = i + 1
  END WHILE
  RETURN lengths
END FUNC

FUNC __canvas_fixedDistLengths() AS List OF Integer
  MUT lengths AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 30
    lengths = collections::append(lengths, 5)
    i = i + 1
  END WHILE
  RETURN lengths
END FUNC

FUNC __canvas_lengthBase(symbol AS Integer) AS Integer
  LET bases AS List OF Integer = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258]
  RETURN collections::getOr(bases, symbol - 257, 0)
END FUNC

FUNC __canvas_lengthExtra(symbol AS Integer) AS Integer
  LET extra AS List OF Integer = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0]
  RETURN collections::getOr(extra, symbol - 257, 0)
END FUNC

FUNC __canvas_distBase(symbol AS Integer) AS Integer
  LET bases AS List OF Integer = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577]
  RETURN collections::getOr(bases, symbol, 0)
END FUNC

FUNC __canvas_distExtra(symbol AS Integer) AS Integer
  LET extra AS List OF Integer = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13]
  RETURN collections::getOr(extra, symbol, 0)
END FUNC"#;

/// The dynamic-block header: the code-length code, then the literal and distance code
/// lengths it encodes (RFC 1951 §3.2.7).
///
/// Returned as `[nextBitPos, hlit, hdist, lengths...]` — one list rather than several
/// out-parameters, because MFBASIC has no out-parameters and threading a cursor through
/// a return value is the honest way to say "this consumed input".
///
/// Code 16 repeats the *previous* length, which is the one place where a malformed
/// stream can ask to repeat a length that does not exist yet; that is refused rather
/// than defaulted, so the failure surfaces at the file rather than in the pixels.
#[rustfmt::skip]
const INFLATE_DYNAMIC: &str =
r#"FUNC __canvas_clOrder() AS List OF Integer
  RETURN [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]
END FUNC

FUNC __canvas_dynamicLengths(data AS List OF Byte, start AS Integer) AS List OF Integer
  MUT pos AS Integer = start
  LET hlit AS Integer = __canvas_bitsAt(data, pos, 5) + 257
  LET hdist AS Integer = __canvas_bitsAt(data, pos + 5, 5) + 1
  LET hclen AS Integer = __canvas_bitsAt(data, pos + 10, 4) + 4
  pos = pos + 14

  MUT clLengths AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 19
    clLengths = collections::append(clLengths, 0)
    i = i + 1
  END WHILE
  LET order AS List OF Integer = __canvas_clOrder()
  i = 0
  WHILE i < hclen
    clLengths = collections::set(clLengths, collections::getOr(order, i, 0), __canvas_bitsAt(data, pos, 3))
    pos = pos + 3
    i = i + 1
  END WHILE

  LET clCounts AS List OF Integer = __canvas_huffCounts(clLengths)
  LET clSymbols AS List OF Integer = __canvas_huffSymbols(clLengths, clCounts)

  MUT lengths AS List OF Integer = []
  LET wanted AS Integer = hlit + hdist
  WHILE len(lengths) < wanted
    LET decoded AS List OF Integer = __canvas_huffDecode(data, pos, clCounts, clSymbols)
    LET symbol AS Integer = collections::getOr(decoded, 0, 0 - 1)
    pos = pos + collections::getOr(decoded, 1, 0)
    IF symbol < 0 THEN
      RETURN []
    END IF
    IF symbol < 16 THEN
      lengths = collections::append(lengths, symbol)
    ELSEIF symbol = 16 THEN
      IF len(lengths) = 0 THEN
        RETURN []
      END IF
      LET previous AS Integer = collections::getOr(lengths, len(lengths) - 1, 0)
      LET repeat AS Integer = __canvas_bitsAt(data, pos, 2) + 3
      pos = pos + 2
      MUT r AS Integer = 0
      WHILE r < repeat
        lengths = collections::append(lengths, previous)
        r = r + 1
      END WHILE
    ELSEIF symbol = 17 THEN
      LET repeat AS Integer = __canvas_bitsAt(data, pos, 3) + 3
      pos = pos + 3
      MUT r AS Integer = 0
      WHILE r < repeat
        lengths = collections::append(lengths, 0)
        r = r + 1
      END WHILE
    ELSE
      LET repeat AS Integer = __canvas_bitsAt(data, pos, 7) + 11
      pos = pos + 7
      MUT r AS Integer = 0
      WHILE r < repeat
        lengths = collections::append(lengths, 0)
        r = r + 1
      END WHILE
    END IF
  END WHILE
  IF len(lengths) > wanted THEN
    RETURN []
  END IF

  MUT out AS List OF Integer = [pos, hlit, hdist]
  FOR EACH l IN lengths
    out = collections::append(out, l)
  NEXT
  RETURN out
END FUNC"#;

/// The block loop.
///
/// `__canvas_inflate` returns the decompressed bytes, or an empty list for a stream it
/// could not decode. An empty result is unambiguous here because every caller in this
/// package is decoding image data, which is never legitimately zero bytes — and the
/// caller turns it into `ErrBadImageFile` rather than into an image of nothing.
///
/// `limit` is the most output the caller can use, and a stream that would produce more
/// is refused at the byte that crosses it (bug-509, DEC-51). It is a refusal, not a
/// truncation: a PNG fixes its filtered size exactly, so a stream carrying more is
/// malformed, and an inflate with no ceiling turned 389 KB of file into 25 GB of
/// output before reporting a 1x1 image. Every one of the three places output grows —
/// a stored block, a literal, a back-reference — checks before it appends, so the
/// output never holds more than `limit` bytes.
#[rustfmt::skip]
const INFLATE: &str =
r#"FUNC __canvas_inflate(data AS List OF Byte, start AS Integer, limit AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT pos AS Integer = start * 8
  LET bits AS Integer = len(data) * 8
  MUT final AS Integer = 0
  WHILE final = 0
    IF pos + 3 > bits THEN
      RETURN []
    END IF
    final = __canvas_bitAt(data, pos)
    LET kind AS Integer = __canvas_bitsAt(data, pos + 1, 2)
    pos = pos + 3
    IF kind = 0 THEN
      ' A stored block restarts at the next byte boundary and carries its length twice,
      ' the second time complemented -- checked, because it is the cheapest place a
      ' truncated file gives itself away.
      pos = ((pos + 7) / 8) * 8
      LET at AS Integer = pos / 8
      LET count AS Integer = toInt(collections::getOr(data, at, toByte(0))) + toInt(collections::getOr(data, at + 1, toByte(0))) * 256
      LET check AS Integer = toInt(collections::getOr(data, at + 2, toByte(0))) + toInt(collections::getOr(data, at + 3, toByte(0))) * 256
      IF count + check <> 65535 THEN
        RETURN []
      END IF
      IF at + 4 + count > len(data) THEN
        RETURN []
      END IF
      IF len(out) + count > limit THEN
        RETURN []
      END IF
      MUT i AS Integer = 0
      WHILE i < count
        out = collections::append(out, collections::getOr(data, at + 4 + i, toByte(0)))
        i = i + 1
      END WHILE
      pos = (at + 4 + count) * 8
    ELSEIF kind = 3 THEN
      RETURN []
    ELSE
      MUT litCounts AS List OF Integer = []
      MUT litSymbols AS List OF Integer = []
      MUT distCounts AS List OF Integer = []
      MUT distSymbols AS List OF Integer = []
      IF kind = 1 THEN
        LET litLengths AS List OF Integer = __canvas_fixedLitLengths()
        litCounts = __canvas_huffCounts(litLengths)
        litSymbols = __canvas_huffSymbols(litLengths, litCounts)
        LET distLengths AS List OF Integer = __canvas_fixedDistLengths()
        distCounts = __canvas_huffCounts(distLengths)
        distSymbols = __canvas_huffSymbols(distLengths, distCounts)
      ELSE
        LET header AS List OF Integer = __canvas_dynamicLengths(data, pos)
        IF len(header) = 0 THEN
          RETURN []
        END IF
        pos = collections::getOr(header, 0, 0)
        LET hlit AS Integer = collections::getOr(header, 1, 0)
        LET hdist AS Integer = collections::getOr(header, 2, 0)
        MUT litLengths AS List OF Integer = []
        MUT distLengths AS List OF Integer = []
        MUT i AS Integer = 0
        WHILE i < hlit
          litLengths = collections::append(litLengths, collections::getOr(header, 3 + i, 0))
          i = i + 1
        END WHILE
        i = 0
        WHILE i < hdist
          distLengths = collections::append(distLengths, collections::getOr(header, 3 + hlit + i, 0))
          i = i + 1
        END WHILE
        litCounts = __canvas_huffCounts(litLengths)
        litSymbols = __canvas_huffSymbols(litLengths, litCounts)
        distCounts = __canvas_huffCounts(distLengths)
        distSymbols = __canvas_huffSymbols(distLengths, distCounts)
      END IF

      MUT running AS Boolean = TRUE
      WHILE running
        IF pos >= bits THEN
          RETURN []
        END IF
        LET decoded AS List OF Integer = __canvas_huffDecode(data, pos, litCounts, litSymbols)
        LET symbol AS Integer = collections::getOr(decoded, 0, 0 - 1)
        pos = pos + collections::getOr(decoded, 1, 0)
        IF symbol < 0 THEN
          RETURN []
        END IF
        IF symbol < 256 THEN
          IF len(out) >= limit THEN
            RETURN []
          END IF
          out = collections::append(out, toByte(symbol))
        ELSEIF symbol = 256 THEN
          running = FALSE
        ELSEIF symbol > 285 THEN
          RETURN []
        ELSE
          LET extraLen AS Integer = __canvas_lengthExtra(symbol)
          LET length AS Integer = __canvas_lengthBase(symbol) + __canvas_bitsAt(data, pos, extraLen)
          pos = pos + extraLen
          LET dDecoded AS List OF Integer = __canvas_huffDecode(data, pos, distCounts, distSymbols)
          LET dSymbol AS Integer = collections::getOr(dDecoded, 0, 0 - 1)
          pos = pos + collections::getOr(dDecoded, 1, 0)
          IF dSymbol < 0 OR dSymbol > 29 THEN
            RETURN []
          END IF
          LET extraDist AS Integer = __canvas_distExtra(dSymbol)
          LET distance AS Integer = __canvas_distBase(dSymbol) + __canvas_bitsAt(data, pos, extraDist)
          pos = pos + extraDist
          LET from AS Integer = len(out) - distance
          IF from < 0 THEN
            RETURN []
          END IF
          IF len(out) + length > limit THEN
            RETURN []
          END IF
          ' Byte at a time, deliberately. A match may overlap its own output -- that is
          ' how DEFLATE spells a run -- so each byte must be able to read one this loop
          ' has already appended.
          MUT c AS Integer = 0
          WHILE c < length
            out = collections::append(out, collections::getOr(out, from + c, toByte(0)))
            c = c + 1
          END WHILE
        END IF
      END WHILE
    END IF
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_zlibInflate(data AS List OF Byte, limit AS Integer) AS List OF Byte
  IF len(data) < 6 THEN
    RETURN []
  END IF
  LET cmf AS Integer = toInt(collections::getOr(data, 0, toByte(0)))
  LET flg AS Integer = toInt(collections::getOr(data, 1, toByte(0)))
  ' Compression method 8 with a window of at most 32 KiB is the only thing PNG permits,
  ' and the header's own check value must hold. A preset dictionary (FDICT) is refused:
  ' PNG forbids it, and accepting one would mean decoding against a dictionary we do
  ' not have.
  IF cmf MOD 16 <> 8 THEN
    RETURN []
  END IF
  IF (cmf * 256 + flg) MOD 31 <> 0 THEN
    RETURN []
  END IF
  IF (flg / 32) MOD 2 <> 0 THEN
    RETURN []
  END IF
  RETURN __canvas_inflate(data, 2, limit)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_inflateBits", INFLATE_BITS));
    pkg.add_helper(RegistryHelper::always("canvas_inflateHuff", INFLATE_HUFF));
    pkg.add_helper(RegistryHelper::always("canvas_inflateFixed", INFLATE_FIXED));
    pkg.add_helper(RegistryHelper::always(
        "canvas_inflateDynamic",
        INFLATE_DYNAMIC,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_inflate", INFLATE));
}
