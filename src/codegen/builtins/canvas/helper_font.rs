//! Reading a TrueType file: big-endian primitives, the table directory, the metric
//! tables, and `cmap`.
//!
//! All of it is MFBASIC, for the reason `.ai/canvas-threading.md` §12 records: the
//! software render is the oracle every GPU backend is gated against, and plan-98-F
//! Phase 1 measured it byte-identical across two ISAs and two operating systems. A
//! per-platform font library would end that. Reading `glyf` ourselves keeps the same
//! code on every target, which is the whole point.
//!
//! **Everything here takes the font's bytes, not a parsed handle.** A TrueType file is
//! already an indexed database — `loca` indexes `glyf`, `cmap` indexes by codepoint —
//! so a parse step would only copy the index into a second index. The lookups are a
//! handful of big-endian reads each; what is worth caching is the *rasterised glyph*,
//! which is plan-98-G Phase 2's job.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Big-endian readers over a `List OF Byte`.
///
/// Every offset in an sfnt is big-endian, and MFBASIC has no fixed-width load, so this
/// is where the format's byte order lives. `__canvas_beS16` exists separately because
/// several fields — `descender`, a glyph's bounding box, `idDelta` — are genuinely
/// signed, and reading them unsigned gives a number near 65536 that looks plausible
/// enough to survive into an offset.
#[rustfmt::skip]
const FONT_BYTES_READ: &str =
r#"FUNC __canvas_beU8(b AS List OF Byte, off AS Integer) AS Integer
  RETURN toInt(collections::getOr(b, off, toByte(0)))
END FUNC

FUNC __canvas_beU16(b AS List OF Byte, off AS Integer) AS Integer
  RETURN __canvas_beU8(b, off) * 256 + __canvas_beU8(b, off + 1)
END FUNC

FUNC __canvas_beS16(b AS List OF Byte, off AS Integer) AS Integer
  LET v AS Integer = __canvas_beU16(b, off)
  IF v >= 32768 THEN
    RETURN v - 65536
  END IF
  RETURN v
END FUNC

FUNC __canvas_beU32(b AS List OF Byte, off AS Integer) AS Integer
  RETURN __canvas_beU16(b, off) * 65536 + __canvas_beU16(b, off + 2)
END FUNC"#;

/// The table directory: find a table by its four-character tag.
///
/// Returns the table's file offset, or `-1` when the font does not carry it. A linear
/// scan rather than the binary search the header's `searchRange`/`entrySelector` fields
/// invite: a font has on the order of a dozen tables, and a wrong binary search over a
/// directory that is *supposed* to be sorted is a class of bug this does not need. The
/// caller checks for `-1` — a missing table is a real thing (`hmtx` without `hhea` is
/// malformed, `cmap` without a usable subtable is a symbol font).
#[rustfmt::skip]
const FONT_TABLE: &str =
r#"FUNC __canvas_fontTable(b AS List OF Byte, tag AS String) AS Integer
  LET numTables AS Integer = __canvas_beU16(b, 4)
  LET want AS List OF Integer = encoding::utf32Encode(tag)
  MUT i AS Integer = 0
  WHILE i < numTables
    LET rec AS Integer = 12 + i * 16
    IF rec + 16 <= len(b) THEN
      MUT same AS Boolean = TRUE
      MUT k AS Integer = 0
      WHILE k < 4
        IF __canvas_beU8(b, rec + k) <> collections::getOr(want, k, 0) THEN
          same = FALSE
        END IF
        k = k + 1
      END WHILE
      IF same THEN
        RETURN __canvas_beU32(b, rec + 8)
      END IF
    END IF
    i = i + 1
  END WHILE
  RETURN 0 - 1
END FUNC"#;

/// The four numbers every measurement needs, read straight from `head` and `hhea`.
///
/// `unitsPerEm` is the font's design grid: every other number here is in those units,
/// and a caller scales by `size / unitsPerEm`. It is *not* always 1000 or 2048 — that
/// is the assumption that makes text the wrong size in exactly one font — so it is
/// read rather than assumed, and a zero or missing `head` yields `0`, which the caller
/// treats as "cannot measure" instead of dividing by it.
#[rustfmt::skip]
const FONT_METRICS: &str =
r#"FUNC __canvas_fontUnitsPerEm(b AS List OF Byte) AS Integer
  LET head AS Integer = __canvas_fontTable(b, "head")
  IF head < 0 THEN
    RETURN 0
  END IF
  RETURN __canvas_beU16(b, head + 18)
END FUNC

FUNC __canvas_fontAscent(b AS List OF Byte) AS Integer
  LET hhea AS Integer = __canvas_fontTable(b, "hhea")
  IF hhea < 0 THEN
    RETURN 0
  END IF
  RETURN __canvas_beS16(b, hhea + 4)
END FUNC

FUNC __canvas_fontDescent(b AS List OF Byte) AS Integer
  LET hhea AS Integer = __canvas_fontTable(b, "hhea")
  IF hhea < 0 THEN
    RETURN 0
  END IF
  RETURN __canvas_beS16(b, hhea + 6)
END FUNC

FUNC __canvas_fontLineGap(b AS List OF Byte) AS Integer
  LET hhea AS Integer = __canvas_fontTable(b, "hhea")
  IF hhea < 0 THEN
    RETURN 0
  END IF
  RETURN __canvas_beS16(b, hhea + 8)
END FUNC"#;

/// A glyph's advance width, in font units.
///
/// `hmtx` holds `numberOfHMetrics` pairs and then degenerates into a bare array of
/// left-side bearings — every glyph past that point shares the *last* pair's advance.
/// That tail is not an edge case: it is how monospaced and CJK fonts store thousands of
/// equal-width glyphs in four bytes, so a reader that indexes `hmtx` naively walks off
/// the table on the most ordinary fonts there are.
#[rustfmt::skip]
const FONT_ADVANCE: &str =
r#"FUNC __canvas_glyphAdvance(b AS List OF Byte, gid AS Integer) AS Integer
  LET hhea AS Integer = __canvas_fontTable(b, "hhea")
  LET hmtx AS Integer = __canvas_fontTable(b, "hmtx")
  IF hhea < 0 OR hmtx < 0 THEN
    RETURN 0
  END IF
  LET count AS Integer = __canvas_beU16(b, hhea + 34)
  IF count = 0 THEN
    RETURN 0
  END IF
  MUT index AS Integer = gid
  IF index >= count THEN
    index = count - 1
  END IF
  RETURN __canvas_beU16(b, hmtx + index * 4)
END FUNC"#;

/// Codepoint to glyph id, through whichever `cmap` subtable the font offers.
///
/// Format 12 is preferred over format 4 because format 4 is 16-bit and cannot name a
/// codepoint above U+FFFF at all; a font that has both is telling us the wide table is
/// the complete one. Both are searched linearly for the same reason the table directory
/// is — the ranges are few, and a wrong binary search here silently returns the wrong
/// glyph rather than failing.
///
/// Glyph `0` is `.notdef` and is the defined answer for "this font has no glyph for
/// that codepoint". Returning it rather than failing is deliberate: a missing glyph
/// should draw the empty box the font provides, not stop the frame.
#[rustfmt::skip]
const FONT_CMAP: &str =
r#"FUNC __canvas_cmapSubtable(b AS List OF Byte) AS Integer
  LET cmap AS Integer = __canvas_fontTable(b, "cmap")
  IF cmap < 0 THEN
    RETURN 0 - 1
  END IF
  LET n AS Integer = __canvas_beU16(b, cmap + 2)
  MUT best AS Integer = 0 - 1
  MUT bestFormat AS Integer = 0 - 1
  MUT i AS Integer = 0
  WHILE i < n
    LET rec AS Integer = cmap + 4 + i * 8
    LET subtable AS Integer = cmap + __canvas_beU32(b, rec + 4)
    LET kind AS Integer = __canvas_beU16(b, subtable)
    IF kind = 12 THEN
      best = subtable
      bestFormat = 12
    END IF
    IF kind = 4 AND bestFormat <> 12 THEN
      best = subtable
      bestFormat = 4
    END IF
    i = i + 1
  END WHILE
  RETURN best
END FUNC

FUNC __canvas_glyphIndex(b AS List OF Byte, cp AS Integer) AS Integer
  LET subtable AS Integer = __canvas_cmapSubtable(b)
  IF subtable < 0 THEN
    RETURN 0
  END IF
  LET kind AS Integer = __canvas_beU16(b, subtable)
  IF kind = 12 THEN
    RETURN __canvas_cmap12(b, subtable, cp)
  END IF
  IF kind = 4 THEN
    RETURN __canvas_cmap4(b, subtable, cp)
  END IF
  RETURN 0
END FUNC

FUNC __canvas_cmap12(b AS List OF Byte, subtable AS Integer, cp AS Integer) AS Integer
  ' `numGroups` is a u32 the file controls, and a group is twelve bytes, so the
  ' subtable's own `length` and the end of the file each bound how many the scan can
  ' be asked for (bug-509, DEC-54). Past the file every "group" reads as zeros through
  ' `getOr` and can match nothing but U+0000 -- to glyph 0, the not-found answer -- so
  ' stopping there changes no lookup. Past `length` the bytes are some other table's,
  ' which is not a cmap at all; FreeType refuses the whole subtable for that, this
  ' keeps the groups the table does declare. Unbounded, one unmapped character was a
  ' scan of 4,294,967,295 groups: 583 s of CPU.
  MUT groups AS Integer = __canvas_beU32(b, subtable + 12)
  LET byLength AS Integer = (__canvas_beU32(b, subtable + 4) - 16) / 12
  LET byFile AS Integer = (len(b) - subtable - 16) / 12
  IF groups > byLength THEN
    groups = byLength
  END IF
  IF groups > byFile THEN
    groups = byFile
  END IF
  MUT i AS Integer = 0
  WHILE i < groups
    LET g AS Integer = subtable + 16 + i * 12
    LET first AS Integer = __canvas_beU32(b, g)
    LET last AS Integer = __canvas_beU32(b, g + 4)
    IF cp >= first AND cp <= last THEN
      RETURN __canvas_beU32(b, g + 8) + (cp - first)
    END IF
    i = i + 1
  END WHILE
  RETURN 0
END FUNC

FUNC __canvas_cmap4(b AS List OF Byte, subtable AS Integer, cp AS Integer) AS Integer
  IF cp > 65535 THEN
    RETURN 0
  END IF
  LET segX2 AS Integer = __canvas_beU16(b, subtable + 6)
  LET segs AS Integer = segX2 / 2
  LET endCodes AS Integer = subtable + 14
  LET startCodes AS Integer = endCodes + segX2 + 2
  LET idDeltas AS Integer = startCodes + segX2
  LET idRanges AS Integer = idDeltas + segX2
  MUT i AS Integer = 0
  WHILE i < segs
    IF cp <= __canvas_beU16(b, endCodes + i * 2) THEN
      LET start AS Integer = __canvas_beU16(b, startCodes + i * 2)
      IF cp < start THEN
        RETURN 0
      END IF
      LET rangeOffset AS Integer = __canvas_beU16(b, idRanges + i * 2)
      LET delta AS Integer = __canvas_beS16(b, idDeltas + i * 2)
      IF rangeOffset = 0 THEN
        RETURN __canvas_wrap16(cp + delta)
      END IF
      LET at AS Integer = idRanges + i * 2 + rangeOffset + (cp - start) * 2
      LET g AS Integer = __canvas_beU16(b, at)
      IF g = 0 THEN
        RETURN 0
      END IF
      RETURN __canvas_wrap16(g + delta)
    END IF
    i = i + 1
  END WHILE
  RETURN 0
END FUNC

FUNC __canvas_wrap16(v AS Integer) AS Integer
  MUT r AS Integer = v MOD 65536
  IF r < 0 THEN
    r = r + 65536
  END IF
  RETURN r
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_fontBytes", FONT_BYTES_READ));
    pkg.add_helper(RegistryHelper::always("canvas_fontTable", FONT_TABLE));
    pkg.add_helper(RegistryHelper::always("canvas_fontMetrics", FONT_METRICS));
    pkg.add_helper(RegistryHelper::always("canvas_glyphAdvance", FONT_ADVANCE));
    pkg.add_helper(RegistryHelper::always("canvas_fontCmap", FONT_CMAP));
}
