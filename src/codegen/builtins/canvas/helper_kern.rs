//! Pair kerning (plan-162-B): the `GPOS` `kern` feature's pair adjustments, or a font's
//! legacy `kern` table, read once at load into the face index's kerning section, and
//! applied by `__canvas_shapeRun`.
//!
//! **What is read.** `GPOS` lookups of type 2 (PairPos, formats 1 and 2), directly or
//! through type 9 (Extension), under the `kern` feature of the `DFLT` script's default
//! LangSys, else `latn`'s, else the first script's. Of each ValueRecord only `XAdvance`
//! is kept; every other field is read only to size the record. A font with a `GPOS`
//! `kern` feature uses it and nothing else. One without falls back to the `kern` table:
//! version 0, format-0 subtables marked horizontal and neither minimum nor cross-stream.
//! That is what HarfBuzz and FreeType do for a legacy font. Apple's version-1 `kern` is
//! not read. Contextual, chained, mark and cursive positioning are not applied; neither
//! is any other feature.
//!
//! **The section.** Header word 9 of the face index (layout version 2) is the section's
//! word offset, or 0 for a font with no kerning. All offsets inside it count from the
//! section's own first word:
//!
//! | Block | Words |
//! |---|---|
//! | section | `lookupCount`, then one offset per lookup |
//! | lookup | `subtableCount`, then one offset per subtable |
//! | pairs subtable | `1, flags, C, C × (firstGid, lastGid, coverageIndex), S, S + 1 pair starts, pairs × (second, xAdvance1, xAdvance2)` |
//! | classes subtable | `2, flags, C, C × coverage runs, R1, R1 × (first, last, class), R2, R2 × runs, class1Count, class2Count, stride, matrix` |
//!
//! `flags` bit 0 is "the pair's second ValueRecord is non-empty": HarfBuzz then moves the
//! walk past the second glyph, so it is not the left glyph of the next pair. Bit 1 is a
//! `kern` subtable's override bit: its value replaces the kerning accumulated for the
//! pair instead of adding to it. A class matrix stores the second glyph's `XAdvance` too
//! (stride 2) only when `valueFormat2` has one.
//!
//! **Hostile tables cost reads, not loops.** Every count the file supplies is clipped to
//! what the file can hold before it bounds a loop (bug-509's rule), and a class matrix
//! that runs past the end of the file is dropped, as HarfBuzz's sanitizer drops it.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Finding a feature's lookups in a `GPOS` or `GSUB` table (`table` is the table's file
/// offset): the script, its default LangSys, the features tagged `feature`, and their
/// lookup indices, ascending and without repeats, which is the order they apply in.
#[rustfmt::skip]
const LAYOUT_FEATURE: &str =
r#"FUNC __canvas_tagIs(b AS List OF Byte, at AS Integer, tag AS String) AS Boolean
  LET want AS List OF Integer = encoding::utf32Encode(tag)
  MUT k AS Integer = 0
  WHILE k < 4
    IF __canvas_beU8(b, at + k) <> collections::getOr(want, k, 0) THEN
      RETURN FALSE
    END IF
    k = k + 1
  END WHILE
  RETURN TRUE
END FUNC

' The Script table for `script`, or -1.
FUNC __canvas_layoutScript(b AS List OF Byte, scripts AS Integer, script AS String) AS Integer
  LET count AS Integer = __canvas_clipCount(b, scripts + 2, __canvas_beU16(b, scripts), 6)
  MUT i AS Integer = 0
  WHILE i < count
    LET rec AS Integer = scripts + 2 + i * 6
    IF __canvas_tagIs(b, rec, script) THEN
      RETURN scripts + __canvas_beU16(b, rec + 4)
    END IF
    i = i + 1
  END WHILE
  RETURN 0 - 1
END FUNC

' The lookups `feature` enables in Script table `script`'s default LangSys.
FUNC __canvas_langSysLookups(b AS List OF Byte, features AS Integer, script AS Integer, feature AS String) AS List OF Integer
  MUT lookups AS List OF Integer = []
  IF script < 0 THEN
    RETURN lookups
  END IF
  LET langSysAt AS Integer = __canvas_beU16(b, script)
  IF langSysAt = 0 THEN
    RETURN lookups
  END IF
  LET langSys AS Integer = script + langSysAt
  LET featureCount AS Integer = __canvas_beU16(b, features)
  MUT indices AS List OF Integer = []
  LET required AS Integer = __canvas_beU16(b, langSys + 2)
  IF required <> 65535 THEN
    indices = collections::append(indices, required)
  END IF
  LET n AS Integer = __canvas_clipCount(b, langSys + 6, __canvas_beU16(b, langSys + 4), 2)
  MUT i AS Integer = 0
  WHILE i < n
    indices = collections::append(indices, __canvas_beU16(b, langSys + 6 + i * 2))
    i = i + 1
  END WHILE
  FOR EACH index IN indices
    IF index < featureCount THEN
      LET rec AS Integer = features + 2 + index * 6
      IF __canvas_tagIs(b, rec, feature) THEN
        LET table AS Integer = features + __canvas_beU16(b, rec + 4)
        LET count AS Integer = __canvas_clipCount(b, table + 4, __canvas_beU16(b, table + 2), 2)
        MUT k AS Integer = 0
        WHILE k < count
          lookups = collections::append(lookups, __canvas_beU16(b, table + 4 + k * 2))
          k = k + 1
        END WHILE
      END IF
    END IF
  NEXT
  RETURN __canvas_sortedDistinct(lookups)
END FUNC

' The lookups `feature` enables, from the first of `DFLT`, `latn` and the first script
' whose default LangSys enables it at all. A table with no such feature answers [].
FUNC __canvas_layoutLookups(b AS List OF Byte, table AS Integer, feature AS String) AS List OF Integer
  LET scripts AS Integer = table + __canvas_beU16(b, table + 4)
  LET features AS Integer = table + __canvas_beU16(b, table + 6)
  MUT candidates AS List OF Integer = [__canvas_layoutScript(b, scripts, "DFLT"), __canvas_layoutScript(b, scripts, "latn")]
  IF __canvas_beU16(b, scripts) > 0 THEN
    candidates = collections::append(candidates, scripts + __canvas_beU16(b, scripts + 6))
  END IF
  FOR EACH script IN candidates
    LET found AS List OF Integer = __canvas_langSysLookups(b, features, script, feature)
    IF len(found) > 0 THEN
      RETURN found
    END IF
  NEXT
  RETURN []
END FUNC

' A count the file supplies, clipped to how many `size`-byte records fit between `at`
' and the end of the file.
FUNC __canvas_clipCount(b AS List OF Byte, at AS Integer, count AS Integer, size AS Integer) AS Integer
  LET room AS Integer = (len(b) - at) / size
  IF room < 0 THEN
    RETURN 0
  END IF
  IF count > room THEN
    RETURN room
  END IF
  RETURN count
END FUNC"#;

/// Coverage tables, ClassDefs and ValueRecords, flattened to runs.
#[rustfmt::skip]
const LAYOUT_COMMON: &str =
r#"' A Coverage table as `[C, C x (firstGid, lastGid, coverageIndex)]`: a glyph in a run
' has coverage index `coverageIndex + gid - firstGid`. Offset 0 is the empty coverage.
FUNC __canvas_coverageRuns(b AS List OF Byte, cov AS Integer, present AS Boolean) AS List OF Integer
  MUT runs AS List OF Integer = []
  IF present THEN
    LET format AS Integer = __canvas_beU16(b, cov)
    IF format = 1 THEN
      LET n AS Integer = __canvas_clipCount(b, cov + 4, __canvas_beU16(b, cov + 2), 2)
      MUT i AS Integer = 0
      WHILE i < n
        LET g AS Integer = __canvas_beU16(b, cov + 4 + i * 2)
        LET m AS Integer = len(runs)
        IF m > 0 AND collections::getOr(runs, m - 2, 0 - 2) = g - 1 THEN
          runs = collections::set(runs, m - 2, g)
        ELSE
          runs = collections::append(runs, g)
          runs = collections::append(runs, g)
          runs = collections::append(runs, i)
        END IF
        i = i + 1
      END WHILE
    END IF
    IF format = 2 THEN
      LET n AS Integer = __canvas_clipCount(b, cov + 4, __canvas_beU16(b, cov + 2), 6)
      MUT i AS Integer = 0
      WHILE i < n
        LET r AS Integer = cov + 4 + i * 6
        runs = collections::append(runs, __canvas_beU16(b, r))
        runs = collections::append(runs, __canvas_beU16(b, r + 2))
        runs = collections::append(runs, __canvas_beU16(b, r + 4))
        i = i + 1
      END WHILE
    END IF
  END IF
  MUT out AS List OF Integer = [len(runs) / 3]
  out = collections::append(out, runs)
  RETURN out
END FUNC

' A ClassDef as `[R, R x (first, last, class)]`; a glyph in no run is class 0. Offset 0
' is the ClassDef that puts every glyph in class 0.
FUNC __canvas_classRuns(b AS List OF Byte, cd AS Integer, present AS Boolean) AS List OF Integer
  MUT runs AS List OF Integer = []
  IF present THEN
    LET format AS Integer = __canvas_beU16(b, cd)
    IF format = 1 THEN
      LET start AS Integer = __canvas_beU16(b, cd + 2)
      LET n AS Integer = __canvas_clipCount(b, cd + 6, __canvas_beU16(b, cd + 4), 2)
      MUT i AS Integer = 0
      WHILE i < n
        LET g AS Integer = start + i
        LET c AS Integer = __canvas_beU16(b, cd + 6 + i * 2)
        IF c <> 0 THEN
          LET m AS Integer = len(runs)
          IF m > 0 AND collections::getOr(runs, m - 2, 0 - 2) = g - 1 AND collections::getOr(runs, m - 1, 0 - 1) = c THEN
            runs = collections::set(runs, m - 2, g)
          ELSE
            runs = collections::append(runs, g)
            runs = collections::append(runs, g)
            runs = collections::append(runs, c)
          END IF
        END IF
        i = i + 1
      END WHILE
    END IF
    IF format = 2 THEN
      LET n AS Integer = __canvas_clipCount(b, cd + 4, __canvas_beU16(b, cd + 2), 6)
      MUT i AS Integer = 0
      WHILE i < n
        LET r AS Integer = cd + 4 + i * 6
        runs = collections::append(runs, __canvas_beU16(b, r))
        runs = collections::append(runs, __canvas_beU16(b, r + 2))
        runs = collections::append(runs, __canvas_beU16(b, r + 4))
        i = i + 1
      END WHILE
    END IF
  END IF
  MUT out AS List OF Integer = [len(runs) / 3]
  out = collections::append(out, runs)
  RETURN out
END FUNC

' Bit `bit` (0 is the lowest) of `v`.
FUNC __canvas_bitOf(v AS Integer, bit AS Integer) AS Integer
  MUT x AS Integer = v
  MUT k AS Integer = 0
  WHILE k < bit
    x = x / 2
    k = k + 1
  END WHILE
  RETURN x MOD 2
END FUNC

' The bytes in a ValueRecord of `format`: two per set bit of the low byte. The device
' and variation offsets (bits 4-7) count; they are never followed.
FUNC __canvas_valueSize(format AS Integer) AS Integer
  MUT size AS Integer = 0
  MUT k AS Integer = 0
  WHILE k < 8
    size = size + 2 * __canvas_bitOf(format, k)
    k = k + 1
  END WHILE
  RETURN size
END FUNC

' The ValueRecord at `at` of `format`'s XAdvance, or 0 when it has none: XAdvance is bit
' 2, after XPlacement (bit 0) and YPlacement (bit 1) when they are present.
FUNC __canvas_valueXAdvance(b AS List OF Byte, at AS Integer, format AS Integer) AS Integer
  IF __canvas_bitOf(format, 2) = 0 THEN
    RETURN 0
  END IF
  RETURN __canvas_beS16(b, at + 2 * (__canvas_bitOf(format, 0) + __canvas_bitOf(format, 1)))
END FUNC

' `[count, off_0, ...]` then the parts, each offset counted from this block's first
' word: how the section holds its lookups and a lookup its subtables.
FUNC __canvas_offsetBlock(parts AS List OF List OF Integer) AS List OF Integer
  MUT out AS List OF Integer = [len(parts)]
  MUT at AS Integer = 1 + len(parts)
  FOR EACH part IN parts
    out = collections::append(out, at)
    at = at + len(part)
  NEXT
  FOR EACH part IN parts
    out = collections::append(out, part)
  NEXT
  RETURN out
END FUNC"#;

/// Building the kerning section at load.
#[rustfmt::skip]
const KERN_SECTION: &str =
r#"' The kerning section for font bytes `b`, or [] for a font with none. A `GPOS` `kern`
' feature, when the font has one, is the whole answer, even if none of its lookups is a
' pair lookup this reads; the `kern` table is only a legacy font's.
FUNC __canvas_kernSection(b AS List OF Byte) AS List OF Integer
  LET gpos AS Integer = __canvas_fontTable(b, "GPOS")
  IF gpos >= 0 THEN
    LET lookups AS List OF Integer = __canvas_layoutLookups(b, gpos, "kern")
    IF len(lookups) > 0 THEN
      RETURN __canvas_gposKern(b, gpos, lookups)
    END IF
  END IF
  LET kern AS Integer = __canvas_fontTable(b, "kern")
  IF kern >= 0 THEN
    RETURN __canvas_kernTable(b, kern)
  END IF
  RETURN []
END FUNC

FUNC __canvas_gposKern(b AS List OF Byte, gpos AS Integer, lookups AS List OF Integer) AS List OF Integer
  LET list AS Integer = gpos + __canvas_beU16(b, gpos + 8)
  LET count AS Integer = __canvas_beU16(b, list)
  MUT blocks AS List OF List OF Integer = []
  FOR EACH index IN lookups
    IF index < count THEN
      LET lk AS Integer = list + __canvas_beU16(b, list + 2 + index * 2)
      LET kind AS Integer = __canvas_beU16(b, lk)
      LET subs AS Integer = __canvas_clipCount(b, lk + 6, __canvas_beU16(b, lk + 4), 2)
      MUT parts AS List OF List OF Integer = []
      MUT s AS Integer = 0
      WHILE s < subs
        MUT st AS Integer = lk + __canvas_beU16(b, lk + 6 + s * 2)
        MUT k AS Integer = kind
        IF kind = 9 AND __canvas_beU16(b, st) = 1 THEN
          k = __canvas_beU16(b, st + 2)
          st = st + __canvas_beU32(b, st + 4)
        END IF
        IF k = 2 THEN
          LET part AS List OF Integer = __canvas_pairPos(b, st)
          IF len(part) > 0 THEN
            parts = collections::append(parts, part)
          END IF
        END IF
        s = s + 1
      END WHILE
      IF len(parts) > 0 THEN
        blocks = collections::append(blocks, __canvas_offsetBlock(parts))
      END IF
    END IF
  NEXT
  IF len(blocks) = 0 THEN
    RETURN []
  END IF
  RETURN __canvas_offsetBlock(blocks)
END FUNC

FUNC __canvas_pairPos(b AS List OF Byte, st AS Integer) AS List OF Integer
  LET format AS Integer = __canvas_beU16(b, st)
  LET covAt AS Integer = __canvas_beU16(b, st + 2)
  LET vf1 AS Integer = __canvas_beU16(b, st + 4)
  LET vf2 AS Integer = __canvas_beU16(b, st + 6)
  MUT flags AS Integer = 0
  IF vf2 <> 0 THEN
    flags = 1
  END IF
  LET size1 AS Integer = __canvas_valueSize(vf1)
  LET size2 AS Integer = __canvas_valueSize(vf2)
  IF format = 1 THEN
    MUT out AS List OF Integer = [1, flags]
    out = collections::append(out, __canvas_coverageRuns(b, st + covAt, covAt <> 0))
    LET sets AS Integer = __canvas_clipCount(b, st + 10, __canvas_beU16(b, st + 8), 2)
    LET rec AS Integer = 2 + size1 + size2
    MUT starts AS List OF Integer = [0]
    MUT pairs AS List OF Integer = []
    MUT k AS Integer = 0
    WHILE k < sets
      LET ps AS Integer = st + __canvas_beU16(b, st + 10 + k * 2)
      LET n AS Integer = __canvas_clipCount(b, ps + 2, __canvas_beU16(b, ps), rec)
      MUT j AS Integer = 0
      WHILE j < n
        LET r AS Integer = ps + 2 + j * rec
        pairs = collections::append(pairs, __canvas_beU16(b, r))
        pairs = collections::append(pairs, __canvas_valueXAdvance(b, r + 2, vf1))
        pairs = collections::append(pairs, __canvas_valueXAdvance(b, r + 2 + size1, vf2))
        j = j + 1
      END WHILE
      starts = collections::append(starts, len(pairs) / 3)
      k = k + 1
    END WHILE
    out = collections::append(out, sets)
    out = collections::append(out, starts)
    out = collections::append(out, pairs)
    RETURN out
  END IF
  IF format = 2 THEN
    LET cd1 AS Integer = __canvas_beU16(b, st + 8)
    LET cd2 AS Integer = __canvas_beU16(b, st + 10)
    LET c1 AS Integer = __canvas_beU16(b, st + 12)
    LET c2 AS Integer = __canvas_beU16(b, st + 14)
    LET cell AS Integer = size1 + size2
    ' A matrix the file cannot hold is no subtable at all.
    IF st + 16 + c1 * c2 * cell > len(b) THEN
      RETURN []
    END IF
    MUT stride AS Integer = 1
    IF __canvas_bitOf(vf2, 2) = 1 THEN
      stride = 2
    END IF
    MUT out AS List OF Integer = [2, flags]
    out = collections::append(out, __canvas_coverageRuns(b, st + covAt, covAt <> 0))
    out = collections::append(out, __canvas_classRuns(b, st + cd1, cd1 <> 0))
    out = collections::append(out, __canvas_classRuns(b, st + cd2, cd2 <> 0))
    out = collections::append(out, c1)
    out = collections::append(out, c2)
    out = collections::append(out, stride)
    MUT i AS Integer = 0
    WHILE i < c1 * c2
      LET r AS Integer = st + 16 + i * cell
      out = collections::append(out, __canvas_valueXAdvance(b, r, vf1))
      IF stride = 2 THEN
        out = collections::append(out, __canvas_valueXAdvance(b, r + size1, vf2))
      END IF
      i = i + 1
    END WHILE
    RETURN out
  END IF
  RETURN []
END FUNC

' The legacy `kern` table: version 0 only, and of its subtables only format 0 marked
' horizontal (coverage bit 0) and neither minimum (bit 1) nor cross-stream (bit 2).
' Each such subtable is a lookup of its own -- the table's subtables accumulate -- and
' its pairs, sorted by (left, right) as the format requires, flatten to the pairs shape
' with one coverage entry per distinct left glyph.
FUNC __canvas_kernTable(b AS List OF Byte, kern AS Integer) AS List OF Integer
  IF __canvas_beU16(b, kern) <> 0 THEN
    RETURN []
  END IF
  LET tables AS Integer = __canvas_beU16(b, kern + 2)
  MUT blocks AS List OF List OF Integer = []
  MUT at AS Integer = kern + 4
  MUT t AS Integer = 0
  WHILE t < tables AND at + 14 <= len(b)
    LET length AS Integer = __canvas_beU16(b, at + 2)
    LET coverage AS Integer = __canvas_beU16(b, at + 4)
    LET format AS Integer = coverage / 256
    IF format = 0 AND __canvas_bitOf(coverage, 0) = 1 AND __canvas_bitOf(coverage, 1) = 0 AND __canvas_bitOf(coverage, 2) = 0 THEN
      LET n AS Integer = __canvas_clipCount(b, at + 14, __canvas_beU16(b, at + 6), 6)
      MUT runs AS List OF Integer = []
      MUT starts AS List OF Integer = [0]
      MUT pairs AS List OF Integer = []
      MUT lefts AS Integer = 0
      MUT last AS Integer = 0 - 1
      MUT i AS Integer = 0
      WHILE i < n
        LET r AS Integer = at + 14 + i * 6
        LET left AS Integer = __canvas_beU16(b, r)
        IF i > 0 AND left <> last THEN
          starts = collections::append(starts, len(pairs) / 3)
        END IF
        IF i = 0 OR left <> last THEN
          LET m AS Integer = len(runs)
          IF m > 0 AND collections::getOr(runs, m - 2, 0 - 2) = left - 1 THEN
            runs = collections::set(runs, m - 2, left)
          ELSE
            runs = collections::append(runs, left)
            runs = collections::append(runs, left)
            runs = collections::append(runs, lefts)
          END IF
          lefts = lefts + 1
          last = left
        END IF
        pairs = collections::append(pairs, __canvas_beU16(b, r + 2))
        pairs = collections::append(pairs, __canvas_beS16(b, r + 4))
        pairs = collections::append(pairs, 0)
        i = i + 1
      END WHILE
      IF n > 0 THEN
        starts = collections::append(starts, len(pairs) / 3)
        MUT flags AS Integer = 0
        IF __canvas_bitOf(coverage, 3) = 1 THEN
          flags = 2
        END IF
        MUT part AS List OF Integer = [1, flags, len(runs) / 3]
        part = collections::append(part, runs)
        part = collections::append(part, lefts)
        part = collections::append(part, starts)
        part = collections::append(part, pairs)
        blocks = collections::append(blocks, __canvas_offsetBlock([part]))
      END IF
    END IF
    IF length < 6 THEN
      t = tables
    ELSE
      at = at + length
      t = t + 1
    END IF
  END WHILE
  IF len(blocks) = 0 THEN
    RETURN []
  END IF
  RETURN __canvas_offsetBlock(blocks)
END FUNC"#;

/// Reading the section by font handle, through `canvas::faceAt`.
///
/// `__canvas_kernLookup` is one lookup's answer for a pair: its subtables in order, and
/// the first that applies wins. It leaves the two adjustments and the subtable's flags in
/// the globals below, because a `List` returned per pair per lookup would be an allocation
/// in the middle of shaping. Format 1 applies when the right glyph is in the left glyph's
/// pair set. Format 2 applies whenever the left glyph is covered and both classes are in
/// range, even when the cell holds 0, and that stops its lookup there too: the rule
/// HarfBuzz's `PairPosFormat1`/`PairPosFormat2::apply` implement.
#[rustfmt::skip]
const KERN_READ: &str =
r#"MUT __CANVAS_KERN_X1 AS Integer = 0
MUT __CANVAS_KERN_X2 AS Integer = 0
MUT __CANVAS_KERN_FLAGS AS Integer = 0

' The index of the run among `count` (first, last, value) runs at `at` holding `g`, or -1.
FUNC __canvas_faceRunAt(font AS Integer, at AS Integer, count AS Integer, g AS Integer) AS Integer
  MUT lo AS Integer = 0
  MUT hi AS Integer = count - 1
  MUT found AS Integer = 0 - 1
  WHILE lo <= hi
    LET mid AS Integer = (lo + hi) / 2
    IF canvas::faceAt(font, at + mid * 3) <= g THEN
      found = mid
      lo = mid + 1
    ELSE
      hi = mid - 1
    END IF
  END WHILE
  IF found < 0 THEN
    RETURN 0 - 1
  END IF
  IF g > canvas::faceAt(font, at + found * 3 + 1) THEN
    RETURN 0 - 1
  END IF
  RETURN found
END FUNC

' The class `g` has in the `count` class runs at `at`: 0 outside them.
FUNC __canvas_faceClass(font AS Integer, at AS Integer, count AS Integer, g AS Integer) AS Integer
  LET r AS Integer = __canvas_faceRunAt(font, at, count, g)
  IF r < 0 THEN
    RETURN 0
  END IF
  RETURN canvas::faceAt(font, at + r * 3 + 2)
END FUNC

' Does the subtable at word `st` apply to (left, right)? On TRUE the adjustments and its
' flags are in __CANVAS_KERN_X1 / _X2 / _FLAGS.
FUNC __canvas_kernSubtable(font AS Integer, st AS Integer, left AS Integer, right AS Integer) AS Boolean
  LET kind AS Integer = canvas::faceAt(font, st)
  LET covCount AS Integer = canvas::faceAt(font, st + 2)
  LET run AS Integer = __canvas_faceRunAt(font, st + 3, covCount, left)
  IF run < 0 THEN
    RETURN FALSE
  END IF
  LET p AS Integer = st + 3 + covCount * 3
  IF kind = 1 THEN
    LET index AS Integer = canvas::faceAt(font, st + 3 + run * 3 + 2) + left - canvas::faceAt(font, st + 3 + run * 3)
    LET sets AS Integer = canvas::faceAt(font, p)
    IF index >= sets THEN
      RETURN FALSE
    END IF
    LET pairs AS Integer = p + 2 + sets
    MUT lo AS Integer = canvas::faceAt(font, p + 1 + index)
    MUT hi AS Integer = canvas::faceAt(font, p + 2 + index) - 1
    WHILE lo <= hi
      LET mid AS Integer = (lo + hi) / 2
      LET second AS Integer = canvas::faceAt(font, pairs + mid * 3)
      IF second = right THEN
        __CANVAS_KERN_X1 = canvas::faceAt(font, pairs + mid * 3 + 1)
        __CANVAS_KERN_X2 = canvas::faceAt(font, pairs + mid * 3 + 2)
        __CANVAS_KERN_FLAGS = canvas::faceAt(font, st + 1)
        RETURN TRUE
      END IF
      IF second < right THEN
        lo = mid + 1
      ELSE
        hi = mid - 1
      END IF
    END WHILE
    RETURN FALSE
  END IF
  IF kind = 2 THEN
    LET r1 AS Integer = canvas::faceAt(font, p)
    LET class1 AS Integer = __canvas_faceClass(font, p + 1, r1, left)
    LET q AS Integer = p + 1 + r1 * 3
    LET r2 AS Integer = canvas::faceAt(font, q)
    LET class2 AS Integer = __canvas_faceClass(font, q + 1, r2, right)
    LET m AS Integer = q + 1 + r2 * 3
    LET c1 AS Integer = canvas::faceAt(font, m)
    LET c2 AS Integer = canvas::faceAt(font, m + 1)
    LET stride AS Integer = canvas::faceAt(font, m + 2)
    IF class1 >= c1 OR class2 >= c2 THEN
      RETURN FALSE
    END IF
    LET cell AS Integer = m + 3 + (class1 * c2 + class2) * stride
    __CANVAS_KERN_X1 = canvas::faceAt(font, cell)
    __CANVAS_KERN_X2 = 0
    IF stride = 2 THEN
      __CANVAS_KERN_X2 = canvas::faceAt(font, cell + 1)
    END IF
    __CANVAS_KERN_FLAGS = canvas::faceAt(font, st + 1)
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC

' One lookup (at word `lk`) for (left, right): its first applying subtable.
FUNC __canvas_kernLookup(font AS Integer, lk AS Integer, left AS Integer, right AS Integer) AS Boolean
  LET subs AS Integer = canvas::faceAt(font, lk)
  MUT s AS Integer = 0
  WHILE s < subs
    IF __canvas_kernSubtable(font, lk + canvas::faceAt(font, lk + 1 + s), left, right) THEN
      RETURN TRUE
    END IF
    s = s + 1
  END WHILE
  RETURN FALSE
END FUNC

' Apply font `which`'s kerning to the shaped `run`: lookup by lookup, along the run's
' pairs whose two glyphs both come from that font. A pair adds its first adjustment to
' the left glyph's advance and its second to the right glyph's. A `kern` override
' subtable replaces what earlier subtables added to the pair. After a pair whose
' second ValueRecord is non-empty the walk resumes past the right glyph, as HarfBuzz's
' does.
FUNC __canvas_kernRun(font AS Integer, which AS Integer, run AS List OF Integer) AS List OF Integer
  LET section AS Integer = canvas::faceAt(font, __CANVAS_FACE_KERN)
  IF section <= 0 THEN
    RETURN run
  END IF
  MUT out AS List OF Integer = run
  LET n AS Integer = len(run) / 3
  MUT added AS List OF Integer = []
  MUT c AS Integer = 0
  WHILE c < n
    added = collections::append(added, 0)
    c = c + 1
  END WHILE
  LET lookups AS Integer = canvas::faceAt(font, section)
  MUT l AS Integer = 0
  WHILE l < lookups
    LET lk AS Integer = section + canvas::faceAt(font, section + 1 + l)
    MUT i AS Integer = 0
    WHILE i + 1 < n
      MUT stride AS Integer = 1
      IF collections::getOr(out, i * 3, 0 - 1) = which AND collections::getOr(out, i * 3 + 3, 0 - 1) = which THEN
        IF __canvas_kernLookup(font, lk, collections::getOr(out, i * 3 + 1, 0), collections::getOr(out, i * 3 + 4, 0)) THEN
          MUT x1 AS Integer = __CANVAS_KERN_X1
          IF __canvas_bitOf(__CANVAS_KERN_FLAGS, 1) = 1 THEN
            x1 = x1 - collections::getOr(added, i, 0)
          END IF
          out = collections::set(out, i * 3 + 2, collections::getOr(out, i * 3 + 2, 0) + x1)
          added = collections::set(added, i, collections::getOr(added, i, 0) + x1)
          out = collections::set(out, i * 3 + 5, collections::getOr(out, i * 3 + 5, 0) + __CANVAS_KERN_X2)
          IF __canvas_bitOf(__CANVAS_KERN_FLAGS, 0) = 1 THEN
            stride = 2
          END IF
        END IF
      END IF
      i = i + stride
    END WHILE
    l = l + 1
  END WHILE
  RETURN out
END FUNC"#;

/// `__canvas_faceKern`, the isolated pair's kerning — the sum, over the lookups, of each
/// lookup's first applying subtable — exists for the `--debug` face check, which lists it
/// for every pair of the first 64 glyph ids (`MFB_CANVAS_FACE_CHECK`, `kern=`). Shaping
/// walks a whole run (`__canvas_kernRun`), so a normal build has no caller for it.
#[rustfmt::skip]
const FACE_KERN_DEBUG: &str =
r#"FUNC __canvas_faceKern(font AS Integer, left AS Integer, right AS Integer) AS Integer
  __CANVAS_KERN_SUM2 = 0
  LET section AS Integer = canvas::faceAt(font, __CANVAS_FACE_KERN)
  IF section <= 0 THEN
    RETURN 0
  END IF
  MUT sum AS Integer = 0
  LET lookups AS Integer = canvas::faceAt(font, section)
  MUT l AS Integer = 0
  WHILE l < lookups
    IF __canvas_kernLookup(font, section + canvas::faceAt(font, section + 1 + l), left, right) THEN
      IF __canvas_bitOf(__CANVAS_KERN_FLAGS, 1) = 1 THEN
        sum = __CANVAS_KERN_X1
      ELSE
        sum = sum + __CANVAS_KERN_X1
      END IF
      __CANVAS_KERN_SUM2 = __CANVAS_KERN_SUM2 + __CANVAS_KERN_X2
    END IF
    l = l + 1
  END WHILE
  RETURN sum
END FUNC

MUT __CANVAS_KERN_SUM2 AS Integer = 0

' The `kern=` field of the face check: `left:right:x1:x2` for every pair of the first
' 64 glyph ids with a non-zero adjustment, comma-separated.
FUNC __canvas_faceKernText(font AS Integer) AS String
  MUT out AS String = ""
  MUT l AS Integer = 0
  WHILE l < 64
    MUT r AS Integer = 0
    WHILE r < 64
      LET x1 AS Integer = __canvas_faceKern(font, l, r)
      IF x1 <> 0 OR __CANVAS_KERN_SUM2 <> 0 THEN
        IF len(out) > 0 THEN
          out = out & ","
        END IF
        out = out & toString(l) & ":" & toString(r) & ":" & toString(x1) & ":" & toString(__CANVAS_KERN_SUM2)
      END IF
      r = r + 1
    END WHILE
    l = l + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_layoutFeature",
        LAYOUT_FEATURE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_layoutCommon", LAYOUT_COMMON));
    pkg.add_helper(RegistryHelper::always("canvas_kernSection", KERN_SECTION));
    pkg.add_helper(RegistryHelper::always("canvas_kernRead", KERN_READ));
    // A normal build has no caller for `__canvas_faceKern`; its half is empty.
    for helper in RegistryHelper::debug_split("canvas_faceKern", "", FACE_KERN_DEBUG) {
        pkg.add_helper(helper);
    }
}
