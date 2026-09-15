//! `__compress_inflateCore` — the raw DEFLATE (RFC 1951) decoder behind every `compress` decoder.
//!
//! One function owns all decoder state as locals declared together (plan-137-B §4.1): input
//! cursor, LSB-first bit buffer (`bits::` operations, ≤ 56 bits), final-block flag, output list
//! and the current block's decode tables. It returns the output with the byte position just past
//! the DEFLATE data appended as 8 little-endian bytes (§4.1 shape (b)); the framing helpers read
//! and remove it. Both choices were measured in plan-137-B Phase 1 (Corrections).
//!
//! Output is written by in-place append to the local `out`; a back-reference copies byte by byte
//! from `out`, so an overlapping match works by construction. `maxBytes` is checked before every
//! write, so a stream that would exceed it raises `ErrTooLarge` without first building the larger
//! result; malformed input raises `ErrInvalidFormat`. The fixed distance table is 32 five-bit
//! codes (RFC 1951 §3.2.6, zlib `fixedtables`); symbols 30 and 31 are refused when decoded.
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

FUNC __compress_inflateCore(data AS List OF Byte, start AS Integer, maxBytes AS Integer) AS List OF Byte
  ' --- decoder state: every piece of it is a local declared here ---
  LET n AS Integer = len(data)
  MUT pos AS Integer = start
  MUT bitBuf AS Integer = 0
  MUT bitCount AS Integer = 0
  MUT final AS Boolean = FALSE
  MUT out AS List OF Byte = []
  MUT outLen AS Integer = 0
  LET fixedLit AS List OF Integer = __compress_buildTable(__compress_fixedLitLens(), 0, 288, 9, 1)
  ' 32 five-bit codes, as zlib's fixedtables: a complete set; symbols 30 and 31 are refused when decoded.
  LET fixedDist AS List OF Integer = __compress_buildTable(__compress_filled(32, 5), 0, 32, 6, 1)
  MUT litT AS List OF Integer = fixedLit
  MUT distT AS List OF Integer = fixedDist
  ' ---
  WHILE final = FALSE
    IF bitCount < 48 THEN
      WHILE bitCount <= 48
        MUT byte AS Integer = 0
        IF pos < n THEN
          byte = toInt(collections::get(data, pos))
        END IF
        bitBuf = bits::bor(bitBuf, bits::sl(byte, bitCount))
        pos = pos + 1
        bitCount = bitCount + 8
      END WHILE
    END IF
    final = bits::band(bitBuf, 1) = 1
    bitBuf = bits::sr(bitBuf, 1)
    bitCount = bitCount - 1
    LET btype AS Integer = bits::band(bitBuf, 3)
    bitBuf = bits::sr(bitBuf, 2)
    bitCount = bitCount - 2
    IF btype = 0 THEN
      LET drop AS Integer = bitCount MOD 8
      bitBuf = bits::sr(bitBuf, drop)
      bitCount = bitCount - drop
      IF bitCount < 32 THEN
        WHILE bitCount <= 48
          MUT byte AS Integer = 0
          IF pos < n THEN
            byte = toInt(collections::get(data, pos))
          END IF
          bitBuf = bits::bor(bitBuf, bits::sl(byte, bitCount))
          pos = pos + 1
          bitCount = bitCount + 8
        END WHILE
      END IF
      LET storedLen AS Integer = bits::band(bitBuf, 65535)
      bitBuf = bits::sr(bitBuf, 16)
      bitCount = bitCount - 16
      LET storedNlen AS Integer = bits::band(bitBuf, 65535)
      bitBuf = bits::sr(bitBuf, 16)
      bitCount = bitCount - 16
      IF pos > n THEN
        IF bitCount < (pos - n) * 8 THEN
          FAIL error(77050003, "compress: input ends mid-stream")
        END IF
      END IF
      IF storedLen + storedNlen <> 65535 THEN
        FAIL error(77050003, "compress: invalid stored block lengths")
      END IF
      IF outLen + storedLen > maxBytes THEN
        FAIL error(77050027, "compress: output exceeds maxBytes")
      END IF
      IF pos > n THEN
        IF storedLen > bitCount / 8 - (pos - n) THEN
          FAIL error(77050003, "compress: input ends mid-stream")
        END IF
      END IF
      MUT remaining AS Integer = storedLen
      WHILE remaining > 0 AND bitCount >= 8
        out = collections::append(out, toByte(bits::band(bitBuf, 255)))
        bitBuf = bits::sr(bitBuf, 8)
        bitCount = bitCount - 8
        remaining = remaining - 1
      END WHILE
      IF remaining > 0 THEN
        IF pos + remaining > n THEN
          FAIL error(77050003, "compress: input ends mid-stream")
        END IF
        MUT k AS Integer = 0
        WHILE k < remaining
          out = collections::append(out, collections::get(data, pos + k))
          k = k + 1
        END WHILE
        pos = pos + remaining
      END IF
      outLen = outLen + storedLen
    ELSEIF btype = 1 THEN
      litT = fixedLit
      distT = fixedDist
      MUT inBlock AS Boolean = TRUE
      WHILE inBlock
        IF bitCount < 48 THEN
          WHILE bitCount <= 48
            MUT byte AS Integer = 0
            IF pos < n THEN
              byte = toInt(collections::get(data, pos))
            END IF
            bitBuf = bits::bor(bitBuf, bits::sl(byte, bitCount))
            pos = pos + 1
            bitCount = bitCount + 8
          END WHILE
        END IF
        MUT symEntry AS Integer = collections::get(litT, bits::band(bitBuf, 511))
        IF symEntry >= 1048576 THEN
          bitBuf = bits::sr(bitBuf, 9)
          bitCount = bitCount - 9
          LET symSub AS Integer = (symEntry - 1048576) MOD 16
          symEntry = collections::get(litT, (symEntry - 1048576) / 16 + bits::band(bitBuf, bits::sl(1, symSub) - 1))
        END IF
        IF symEntry < 0 THEN
          FAIL error(77050003, "compress: invalid code")
        END IF
        bitBuf = bits::sr(bitBuf, symEntry MOD 16)
        bitCount = bitCount - symEntry MOD 16
        LET sym AS Integer = symEntry / 16
        IF sym < 256 THEN
          IF outLen >= maxBytes THEN
            FAIL error(77050027, "compress: output exceeds maxBytes")
          END IF
          out = collections::append(out, toByte(sym))
          outLen = outLen + 1
        ELSEIF sym = 256 THEN
          inBlock = FALSE
        ELSE
          LET li AS Integer = sym - 257
          IF li >= 29 THEN
            FAIL error(77050003, "compress: invalid length symbol")
          END IF
          LET lx AS Integer = collections::get(__COMPRESS_LEN_EXTRA, li)
          LET length AS Integer = collections::get(__COMPRESS_LEN_BASE, li) + bits::band(bitBuf, bits::sl(1, lx) - 1)
          bitBuf = bits::sr(bitBuf, lx)
          bitCount = bitCount - lx
          MUT dsymEntry AS Integer = collections::get(distT, bits::band(bitBuf, 63))
          IF dsymEntry >= 1048576 THEN
            bitBuf = bits::sr(bitBuf, 6)
            bitCount = bitCount - 6
            LET dsymSub AS Integer = (dsymEntry - 1048576) MOD 16
            dsymEntry = collections::get(distT, (dsymEntry - 1048576) / 16 + bits::band(bitBuf, bits::sl(1, dsymSub) - 1))
          END IF
          IF dsymEntry < 0 THEN
            FAIL error(77050003, "compress: invalid code")
          END IF
          bitBuf = bits::sr(bitBuf, dsymEntry MOD 16)
          bitCount = bitCount - dsymEntry MOD 16
          LET dsym AS Integer = dsymEntry / 16
          IF dsym >= 30 THEN
            FAIL error(77050003, "compress: invalid distance symbol")
          END IF
          LET dx AS Integer = collections::get(__COMPRESS_DIST_EXTRA, dsym)
          LET dist AS Integer = collections::get(__COMPRESS_DIST_BASE, dsym) + bits::band(bitBuf, bits::sl(1, dx) - 1)
          bitBuf = bits::sr(bitBuf, dx)
          bitCount = bitCount - dx
          IF dist > outLen THEN
            FAIL error(77050003, "compress: distance too far back")
          END IF
          IF outLen + length > maxBytes THEN
            FAIL error(77050027, "compress: output exceeds maxBytes")
          END IF
          LET from AS Integer = outLen - dist
          MUT k AS Integer = 0
          WHILE k < length
            out = collections::append(out, collections::get(out, from + k))
            k = k + 1
          END WHILE
          outLen = outLen + length
        END IF
        IF pos > n THEN
          IF bitCount < (pos - n) * 8 THEN
            FAIL error(77050003, "compress: input ends mid-stream")
          END IF
        END IF
      END WHILE
    ELSEIF btype = 2 THEN
      LET hlit AS Integer = bits::band(bitBuf, 31) + 257
      bitBuf = bits::sr(bitBuf, 5)
      bitCount = bitCount - 5
      LET hdist AS Integer = bits::band(bitBuf, 31) + 1
      bitBuf = bits::sr(bitBuf, 5)
      bitCount = bitCount - 5
      LET hclen AS Integer = bits::band(bitBuf, 15) + 4
      bitBuf = bits::sr(bitBuf, 4)
      bitCount = bitCount - 4
      IF hlit > 286 OR hdist > 30 THEN
        FAIL error(77050003, "compress: too many length or distance symbols")
      END IF
      MUT clens AS List OF Integer = __compress_filled(19, 0)
      MUT ci AS Integer = 0
      WHILE ci < hclen
        IF bitCount < 3 THEN
          WHILE bitCount <= 48
            MUT byte AS Integer = 0
            IF pos < n THEN
              byte = toInt(collections::get(data, pos))
            END IF
            bitBuf = bits::bor(bitBuf, bits::sl(byte, bitCount))
            pos = pos + 1
            bitCount = bitCount + 8
          END WHILE
        END IF
        clens = collections::set(clens, collections::get(__COMPRESS_CL_ORDER, ci), bits::band(bitBuf, 7))
        bitBuf = bits::sr(bitBuf, 3)
        bitCount = bitCount - 3
        ci = ci + 1
      END WHILE
      LET clT AS List OF Integer = __compress_buildTable(clens, 0, 19, 7, 0)
      LET total AS Integer = hlit + hdist
      MUT lens AS List OF Integer = []
      MUT have AS Integer = 0
      WHILE have < total
        IF bitCount < 16 THEN
          WHILE bitCount <= 48
            MUT byte AS Integer = 0
            IF pos < n THEN
              byte = toInt(collections::get(data, pos))
            END IF
            bitBuf = bits::bor(bitBuf, bits::sl(byte, bitCount))
            pos = pos + 1
            bitCount = bitCount + 8
          END WHILE
        END IF
        MUT clEntry AS Integer = collections::get(clT, bits::band(bitBuf, 127))
        IF clEntry >= 1048576 THEN
          bitBuf = bits::sr(bitBuf, 7)
          bitCount = bitCount - 7
          LET clSub AS Integer = (clEntry - 1048576) MOD 16
          clEntry = collections::get(clT, (clEntry - 1048576) / 16 + bits::band(bitBuf, bits::sl(1, clSub) - 1))
        END IF
        IF clEntry < 0 THEN
          FAIL error(77050003, "compress: invalid code")
        END IF
        bitBuf = bits::sr(bitBuf, clEntry MOD 16)
        bitCount = bitCount - clEntry MOD 16
        LET cl AS Integer = clEntry / 16
        IF cl < 16 THEN
          lens = collections::append(lens, cl)
          have = have + 1
        ELSE
          MUT rep AS Integer = 0
          MUT value AS Integer = 0
          IF cl = 16 THEN
            IF have = 0 THEN
              FAIL error(77050003, "compress: invalid bit length repeat")
            END IF
            value = collections::get(lens, have - 1)
            rep = 3 + bits::band(bitBuf, 3)
            bitBuf = bits::sr(bitBuf, 2)
            bitCount = bitCount - 2
          ELSEIF cl = 17 THEN
            rep = 3 + bits::band(bitBuf, 7)
            bitBuf = bits::sr(bitBuf, 3)
            bitCount = bitCount - 3
          ELSE
            rep = 11 + bits::band(bitBuf, 127)
            bitBuf = bits::sr(bitBuf, 7)
            bitCount = bitCount - 7
          END IF
          IF have + rep > total THEN
            FAIL error(77050003, "compress: invalid bit length repeat")
          END IF
          MUT r AS Integer = 0
          WHILE r < rep
            lens = collections::append(lens, value)
            r = r + 1
          END WHILE
          have = have + rep
        END IF
        IF pos > n THEN
          IF bitCount < (pos - n) * 8 THEN
            FAIL error(77050003, "compress: input ends mid-stream")
          END IF
        END IF
      END WHILE
      IF collections::get(lens, 256) = 0 THEN
        FAIL error(77050003, "compress: missing end-of-block code")
      END IF
      litT = __compress_buildTable(lens, 0, hlit, 9, 1)
      distT = __compress_buildTable(lens, hlit, hdist, 6, 1)
      MUT inBlock AS Boolean = TRUE
      WHILE inBlock
        IF bitCount < 48 THEN
          WHILE bitCount <= 48
            MUT byte AS Integer = 0
            IF pos < n THEN
              byte = toInt(collections::get(data, pos))
            END IF
            bitBuf = bits::bor(bitBuf, bits::sl(byte, bitCount))
            pos = pos + 1
            bitCount = bitCount + 8
          END WHILE
        END IF
        MUT symEntry AS Integer = collections::get(litT, bits::band(bitBuf, 511))
        IF symEntry >= 1048576 THEN
          bitBuf = bits::sr(bitBuf, 9)
          bitCount = bitCount - 9
          LET symSub AS Integer = (symEntry - 1048576) MOD 16
          symEntry = collections::get(litT, (symEntry - 1048576) / 16 + bits::band(bitBuf, bits::sl(1, symSub) - 1))
        END IF
        IF symEntry < 0 THEN
          FAIL error(77050003, "compress: invalid code")
        END IF
        bitBuf = bits::sr(bitBuf, symEntry MOD 16)
        bitCount = bitCount - symEntry MOD 16
        LET sym AS Integer = symEntry / 16
        IF sym < 256 THEN
          IF outLen >= maxBytes THEN
            FAIL error(77050027, "compress: output exceeds maxBytes")
          END IF
          out = collections::append(out, toByte(sym))
          outLen = outLen + 1
        ELSEIF sym = 256 THEN
          inBlock = FALSE
        ELSE
          LET li AS Integer = sym - 257
          IF li >= 29 THEN
            FAIL error(77050003, "compress: invalid length symbol")
          END IF
          LET lx AS Integer = collections::get(__COMPRESS_LEN_EXTRA, li)
          LET length AS Integer = collections::get(__COMPRESS_LEN_BASE, li) + bits::band(bitBuf, bits::sl(1, lx) - 1)
          bitBuf = bits::sr(bitBuf, lx)
          bitCount = bitCount - lx
          MUT dsymEntry AS Integer = collections::get(distT, bits::band(bitBuf, 63))
          IF dsymEntry >= 1048576 THEN
            bitBuf = bits::sr(bitBuf, 6)
            bitCount = bitCount - 6
            LET dsymSub AS Integer = (dsymEntry - 1048576) MOD 16
            dsymEntry = collections::get(distT, (dsymEntry - 1048576) / 16 + bits::band(bitBuf, bits::sl(1, dsymSub) - 1))
          END IF
          IF dsymEntry < 0 THEN
            FAIL error(77050003, "compress: invalid code")
          END IF
          bitBuf = bits::sr(bitBuf, dsymEntry MOD 16)
          bitCount = bitCount - dsymEntry MOD 16
          LET dsym AS Integer = dsymEntry / 16
          IF dsym >= 30 THEN
            FAIL error(77050003, "compress: invalid distance symbol")
          END IF
          LET dx AS Integer = collections::get(__COMPRESS_DIST_EXTRA, dsym)
          LET dist AS Integer = collections::get(__COMPRESS_DIST_BASE, dsym) + bits::band(bitBuf, bits::sl(1, dx) - 1)
          bitBuf = bits::sr(bitBuf, dx)
          bitCount = bitCount - dx
          IF dist > outLen THEN
            FAIL error(77050003, "compress: distance too far back")
          END IF
          IF outLen + length > maxBytes THEN
            FAIL error(77050027, "compress: output exceeds maxBytes")
          END IF
          LET from AS Integer = outLen - dist
          MUT k AS Integer = 0
          WHILE k < length
            out = collections::append(out, collections::get(out, from + k))
            k = k + 1
          END WHILE
          outLen = outLen + length
        END IF
        IF pos > n THEN
          IF bitCount < (pos - n) * 8 THEN
            FAIL error(77050003, "compress: input ends mid-stream")
          END IF
        END IF
      END WHILE
    ELSE
      FAIL error(77050003, "compress: invalid block type")
    END IF
  END WHILE
  IF pos > n THEN
    IF bitCount < (pos - n) * 8 THEN
      FAIL error(77050003, "compress: input ends mid-stream")
    END IF
  END IF
  MUT endPos AS Integer = pos - bitCount / 8
  MUT t AS Integer = 0
  WHILE t < 8
    out = collections::append(out, toByte(endPos MOD 256))
    endPos = endPos / 256
    t = t + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_inflate_core",
        gate: HelperGate::WhenUsed(&["inflate", "zlibDecode", "gzipDecode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
