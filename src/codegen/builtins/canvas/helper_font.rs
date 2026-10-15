//! Reading a TrueType file: big-endian primitives, the table directory, the metric
//! tables, and `cmap`.
//!
//! All of it is MFBASIC, for the reason `.ai/canvas-threading.md` §12 records: the
//! software render is the oracle every GPU backend is gated against, and plan-98-F
//! Phase 1 measured it byte-identical across two ISAs and two operating systems. A
//! per-platform font library would end that. Reading `glyf` ourselves keeps the same
//! code on every target, which is the whole point.
//!
//! **Text reads a parsed face index by handle; only the loader reads the bytes.** A
//! TrueType file is already an indexed database, and the first design read it in place
//! for that reason. But the bytes are the worker's resource, so every text path had to
//! copy the whole file to read four numbers and a `cmap` (`canvas::fontBlobUnchecked`
//! records why it copies), once per string. plan-162-A parses the part every string
//! needs — metrics, `cmap`, advances — once, at load (`__canvas_faceIndex`), and text
//! code reads it a word at a time through `canvas::faceAt`. The byte readers below are
//! what builds the index; outlines (`helper_glyph.rs`) are the one per-glyph read that
//! still needs the file.

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
/// `__canvas_faceTable` reads the directory at `dir` — `0` for an ordinary sfnt, or one
/// face's directory inside a collection, whose table offsets still count from the start
/// of the file (plan-148-A). Every renderer-side reader goes through `__canvas_fontTable`,
/// which is the directory at `0`: a loaded font is always a standalone face, because the
/// loader lifts a collection's face out before stamping the resource.
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
  RETURN __canvas_faceTable(b, 0, tag)
END FUNC

FUNC __canvas_faceTable(b AS List OF Byte, dir AS Integer, tag AS String) AS Integer
  LET numTables AS Integer = __canvas_beU16(b, dir + 4)
  LET want AS List OF Integer = encoding::utf32Encode(tag)
  MUT i AS Integer = 0
  WHILE i < numTables
    LET rec AS Integer = dir + 12 + i * 16
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

/// The `cmap` rules the face index is built from: which subtable a font offers, and how
/// one format-4 segment and the format-12 group count are read.
///
/// Format 12 is preferred over format 4 because format 4 is 16-bit and cannot name a
/// codepoint above U+FFFF at all; a font that has both is telling us the wide table is
/// the complete one.
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

' How many format-12 groups a scan may read. `numGroups` is a u32 the file controls, and
' a group is twelve bytes, so the subtable's own `length` and the end of the file each
' bound how many the scan can be asked for (bug-509, DEC-54). Past the file every
' "group" reads as zeros through `getOr` and can match nothing but U+0000 -- to glyph 0,
' the not-found answer -- so stopping there changes no lookup. Past `length` the bytes
' are some other table's, which is not a cmap at all; FreeType refuses the whole
' subtable for that, this keeps the groups the table does declare. Unbounded, one
' unmapped character was a scan of 4,294,967,295 groups: 583 s of CPU.
FUNC __canvas_cmap12Groups(b AS List OF Byte, subtable AS Integer) AS Integer
  MUT groups AS Integer = __canvas_beU32(b, subtable + 12)
  LET byLength AS Integer = (__canvas_beU32(b, subtable + 4) - 16) / 12
  LET byFile AS Integer = (len(b) - subtable - 16) / 12
  IF groups > byLength THEN
    groups = byLength
  END IF
  IF groups > byFile THEN
    groups = byFile
  END IF
  RETURN groups
END FUNC

' Segment `i`'s answer for `cp`, given that `i` is the first segment whose endCode is at
' or past `cp`: the one rule the face index's run builder and the `--debug` per-codepoint
' lookup `__canvas_cmap4` share, so the two cannot disagree on an `idRangeOffset` segment.
FUNC __canvas_cmap4Segment(b AS List OF Byte, subtable AS Integer, i AS Integer, cp AS Integer) AS Integer
  LET segX2 AS Integer = __canvas_beU16(b, subtable + 6)
  LET endCodes AS Integer = subtable + 14
  LET startCodes AS Integer = endCodes + segX2 + 2
  LET idDeltas AS Integer = startCodes + segX2
  LET idRanges AS Integer = idDeltas + segX2
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
END FUNC

FUNC __canvas_wrap16(v AS Integer) AS Integer
  MUT r AS Integer = v MOD 65536
  IF r < 0 THEN
    r = r + 65536
  END IF
  RETURN r
END FUNC"#;

/// The face index (plan-162-A): everything text needs from a font but its outlines,
/// parsed **once**, when the font loads, into a flat `List OF Integer` the `Font`
/// resource owns (`gen_font.rs:FONT_FACE`) and the font table publishes beside the bytes.
///
/// Layout version 2, header words then sections at the offsets the header names:
///
/// | Word | Meaning |
/// |---|---|
/// | 0 | layout version (2) |
/// | 1 | `head.unitsPerEm`, or 0 for a font with no `head` |
/// | 2, 3, 4 | `hhea` ascender, descender (stored negative, as in the file), lineGap |
/// | 5, 6 | cmap run count `R`, and the word the runs start at |
/// | 7, 8 | advance count `H` (`numberOfHMetrics`), and the word they start at |
/// | 9 | the kerning section's first word, or 0 for a font with no kerning (plan-162-B, `helper_kern.rs`) |
///
/// Each later letter of plan-162 appends its section and bumps the version: 1 was the
/// metrics, `cmap` and advances alone.
///
/// A cmap **run** is `(firstCp, lastCp, delta)`: every codepoint in it maps to
/// `cp + delta`, the runs are sorted by `firstCp` and never overlap, and a codepoint in
/// none maps to glyph 0. The advances are `hmtx`'s advance column; a glyph at or past
/// `H` takes word `H - 1`, which is `__canvas_glyphAdvance`'s shared trailing advance.
///
/// **It is built by the byte readers, not beside them.** The metric words are the
/// `__canvas_font*` answers, a format-4 run is a span of codepoints over which
/// `__canvas_cmap4Segment` -- the evaluator `__canvas_cmap4` itself calls -- answers with
/// one constant `gid - cp`, and a format-12 run is a group, read with the same
/// `__canvas_cmap12Groups` bound. So the index says exactly what the per-call readers
/// said, for every codepoint, and `rt_canvas_font`'s `face_index` sweep holds it to that.
///
/// Format 4 is resolved by the rule `__canvas_cmap4` scans by: a codepoint belongs to
/// the **first** segment whose endCode reaches it, so segment `i` answers exactly the
/// codepoints above every earlier segment's endCode and at or below its own. Walking the
/// segments in file order with that high-water mark visits each of the 65,536 codepoints
/// at most once, whatever order a malformed file puts its segments in.
///
/// Format 12 is "the first group that contains it". Every real font lists its groups
/// sorted and disjoint (all 105 format-12 faces plan-162-A Phase 1 measured), and then a
/// run is a group. A file whose groups overlap or go backwards takes
/// `__canvas_cmap12Painted`, which settles the same rule in O(n log n): a hostile table
/// costs a sort at load, never a quadratic loop (bug-509's principle).
#[rustfmt::skip]
const FACE_INDEX: &str =
r#"LET __CANVAS_FACE_UPEM AS Integer = 1
LET __CANVAS_FACE_ASCENT AS Integer = 2
LET __CANVAS_FACE_DESCENT AS Integer = 3
LET __CANVAS_FACE_LINEGAP AS Integer = 4
LET __CANVAS_FACE_RUNS AS Integer = 5
LET __CANVAS_FACE_RUNS_AT AS Integer = 6
LET __CANVAS_FACE_ADVANCES AS Integer = 7
LET __CANVAS_FACE_ADVANCES_AT AS Integer = 8
LET __CANVAS_FACE_KERN AS Integer = 9
LET __CANVAS_FACE_HEADER AS Integer = 10

FUNC __canvas_faceIndex(b AS List OF Byte) AS List OF Integer
  LET runs AS List OF Integer = __canvas_cmapRuns(b)
  LET advances AS List OF Integer = __canvas_faceAdvances(b)
  LET kerning AS List OF Integer = __canvas_kernSection(b)
  LET body AS Integer = __CANVAS_FACE_HEADER + len(runs) + len(advances)
  MUT kernAt AS Integer = 0
  IF len(kerning) > 0 THEN
    kernAt = body
  END IF
  MUT out AS List OF Integer = [2, __canvas_fontUnitsPerEm(b), __canvas_fontAscent(b), __canvas_fontDescent(b), __canvas_fontLineGap(b), len(runs) / 3, __CANVAS_FACE_HEADER, len(advances), __CANVAS_FACE_HEADER + len(runs), kernAt]
  out = collections::append(out, runs)
  out = collections::append(out, advances)
  out = collections::append(out, kerning)
  RETURN out
END FUNC

FUNC __canvas_faceAdvances(b AS List OF Byte) AS List OF Integer
  MUT out AS List OF Integer = []
  LET hhea AS Integer = __canvas_fontTable(b, "hhea")
  LET hmtx AS Integer = __canvas_fontTable(b, "hmtx")
  IF hhea < 0 OR hmtx < 0 THEN
    RETURN out
  END IF
  LET count AS Integer = __canvas_beU16(b, hhea + 34)
  MUT i AS Integer = 0
  WHILE i < count
    out = collections::append(out, __canvas_beU16(b, hmtx + i * 4))
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_cmapRuns(b AS List OF Byte) AS List OF Integer
  LET subtable AS Integer = __canvas_cmapSubtable(b)
  IF subtable < 0 THEN
    RETURN []
  END IF
  LET kind AS Integer = __canvas_beU16(b, subtable)
  IF kind = 12 THEN
    RETURN __canvas_cmap12Runs(b, subtable)
  END IF
  IF kind = 4 THEN
    RETURN __canvas_cmap4Runs(b, subtable)
  END IF
  RETURN []
END FUNC

FUNC __canvas_cmap4Runs(b AS List OF Byte, subtable AS Integer) AS List OF Integer
  LET segX2 AS Integer = __canvas_beU16(b, subtable + 6)
  LET segs AS Integer = segX2 / 2
  LET endCodes AS Integer = subtable + 14
  LET startCodes AS Integer = endCodes + segX2 + 2
  MUT runs AS List OF Integer = []
  ' The highest codepoint an earlier segment already answers.
  MUT answered AS Integer = 0 - 1
  MUT i AS Integer = 0
  WHILE i < segs
    LET last AS Integer = __canvas_beU16(b, endCodes + i * 2)
    IF last > answered THEN
      ' Below `start` the segment answers 0, which is no run.
      MUT cp AS Integer = answered + 1
      LET start AS Integer = __canvas_beU16(b, startCodes + i * 2)
      IF cp < start THEN
        cp = start
      END IF
      WHILE cp <= last
        LET g AS Integer = __canvas_cmap4Segment(b, subtable, i, cp)
        IF g <> 0 THEN
          LET n AS Integer = len(runs)
          IF n > 0 AND collections::getOr(runs, n - 2, 0 - 2) = cp - 1 AND collections::getOr(runs, n - 1, 0) = g - cp THEN
            runs = collections::set(runs, n - 2, cp)
          ELSE
            runs = collections::append(runs, cp)
            runs = collections::append(runs, cp)
            runs = collections::append(runs, g - cp)
          END IF
        END IF
        cp = cp + 1
      END WHILE
      answered = last
    END IF
    i = i + 1
  END WHILE
  RETURN runs
END FUNC

FUNC __canvas_cmap12Runs(b AS List OF Byte, subtable AS Integer) AS List OF Integer
  LET groups AS Integer = __canvas_cmap12Groups(b, subtable)
  MUT runs AS List OF Integer = []
  ' The highest codepoint a group has claimed so far.
  MUT claimed AS Integer = 0 - 1
  MUT i AS Integer = 0
  WHILE i < groups
    LET g AS Integer = subtable + 16 + i * 12
    LET first AS Integer = __canvas_beU32(b, g)
    LET last AS Integer = __canvas_beU32(b, g + 4)
    ' A group whose last is before its first contains nothing, as `__canvas_cmap12` reads it.
    IF first <= last THEN
      IF first <= claimed THEN
        RETURN __canvas_cmap12Painted(b, subtable, groups)
      END IF
      runs = collections::append(runs, first)
      runs = collections::append(runs, last)
      runs = collections::append(runs, __canvas_beU32(b, g + 8) - first)
      claimed = last
    END IF
    i = i + 1
  END WHILE
  RETURN runs
END FUNC

' Format 12 whose groups overlap or go backwards: each codepoint still belongs to the
' first group that contains it. The groups' ends cut the codepoint line into pieces no
' group boundary crosses; the groups then paint the pieces in file order, and a piece
' keeps the first paint it gets. `onward` is the next unpainted piece at or after each
' piece -- a union-find with path compression -- so every piece is painted once and a
' group whose range is already painted costs almost nothing to skip.
FUNC __canvas_cmap12Painted(b AS List OF Byte, subtable AS Integer, groups AS Integer) AS List OF Integer
  MUT marks AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < groups
    LET g AS Integer = subtable + 16 + i * 12
    LET first AS Integer = __canvas_beU32(b, g)
    LET last AS Integer = __canvas_beU32(b, g + 4)
    IF first <= last THEN
      marks = collections::append(marks, first)
      marks = collections::append(marks, last + 1)
    END IF
    i = i + 1
  END WHILE
  LET cuts AS List OF Integer = __canvas_sortedDistinct(marks)
  LET pieces AS Integer = len(cuts) - 1
  MUT owner AS List OF Integer = []
  MUT onward AS List OF Integer = []
  MUT k AS Integer = 0
  WHILE k <= pieces
    owner = collections::append(owner, 0 - 1)
    onward = collections::append(onward, k)
    k = k + 1
  END WHILE
  i = 0
  WHILE i < groups
    LET g AS Integer = subtable + 16 + i * 12
    LET first AS Integer = __canvas_beU32(b, g)
    LET last AS Integer = __canvas_beU32(b, g + 4)
    IF first <= last THEN
      LET stop AS Integer = __canvas_cutAt(cuts, last + 1)
      MUT at AS Integer = __canvas_cutAt(cuts, first)
      MUT painting AS Boolean = TRUE
      WHILE painting
        MUT root AS Integer = at
        WHILE collections::getOr(onward, root, pieces) <> root
          root = collections::getOr(onward, root, pieces)
        END WHILE
        MUT walk AS Integer = at
        WHILE walk <> root
          LET up AS Integer = collections::getOr(onward, walk, pieces)
          onward = collections::set(onward, walk, root)
          walk = up
        END WHILE
        IF root < stop THEN
          owner = collections::set(owner, root, i)
          onward = collections::set(onward, root, root + 1)
          at = root + 1
        ELSE
          painting = FALSE
        END IF
      END WHILE
    END IF
    i = i + 1
  END WHILE
  MUT runs AS List OF Integer = []
  k = 0
  WHILE k < pieces
    LET o AS Integer = collections::getOr(owner, k, 0 - 1)
    IF o >= 0 THEN
      LET g AS Integer = subtable + 16 + o * 12
      LET delta AS Integer = __canvas_beU32(b, g + 8) - __canvas_beU32(b, g)
      LET lo AS Integer = collections::getOr(cuts, k, 0)
      LET hi AS Integer = collections::getOr(cuts, k + 1, 0) - 1
      LET n AS Integer = len(runs)
      IF n > 0 AND collections::getOr(runs, n - 2, 0 - 2) = lo - 1 AND collections::getOr(runs, n - 1, 0) = delta THEN
        runs = collections::set(runs, n - 2, hi)
      ELSE
        runs = collections::append(runs, lo)
        runs = collections::append(runs, hi)
        runs = collections::append(runs, delta)
      END IF
    END IF
    k = k + 1
  END WHILE
  RETURN runs
END FUNC

' `xs` ascending with repeats dropped: a bottom-up merge sort, then one pass. Written
' here rather than through `collections::sort`, whose body the canvas helpers cannot
' reach -- it is injected only for a program that calls it.
FUNC __canvas_sortedDistinct(xs AS List OF Integer) AS List OF Integer
  LET n AS Integer = len(xs)
  MUT from AS List OF Integer = xs
  MUT width AS Integer = 1
  WHILE width < n
    MUT into AS List OF Integer = []
    MUT lo AS Integer = 0
    WHILE lo < n
      LET mid AS Integer = __canvas_minI(lo + width, n)
      LET hi AS Integer = __canvas_minI(lo + width * 2, n)
      MUT a AS Integer = lo
      MUT c AS Integer = mid
      WHILE a < mid OR c < hi
        IF c >= hi OR (a < mid AND collections::getOr(from, a, 0) <= collections::getOr(from, c, 0)) THEN
          into = collections::append(into, collections::getOr(from, a, 0))
          a = a + 1
        ELSE
          into = collections::append(into, collections::getOr(from, c, 0))
          c = c + 1
        END IF
      END WHILE
      lo = hi
    END WHILE
    from = into
    width = width * 2
  END WHILE
  MUT out AS List OF Integer = []
  FOR EACH x IN from
    IF len(out) = 0 OR collections::getOr(out, len(out) - 1, 0) <> x THEN
      out = collections::append(out, x)
    END IF
  NEXT
  RETURN out
END FUNC

' The index of `v` in the sorted, distinct `cuts`, which holds it.
FUNC __canvas_cutAt(cuts AS List OF Integer, v AS Integer) AS Integer
  MUT lo AS Integer = 0
  MUT hi AS Integer = len(cuts) - 1
  WHILE lo < hi
    LET mid AS Integer = (lo + hi) / 2
    IF collections::getOr(cuts, mid, 0) < v THEN
      lo = mid + 1
    ELSE
      hi = mid
    END IF
  END WHILE
  RETURN lo
END FUNC"#;

/// Reading the face index by font **handle**, through `canvas::faceAt`, which reads one
/// word in place and never copies the block (`gen_font_table.rs:lower_face_at`).
///
/// A handle with no published font reads `0` everywhere — no runs, no advances, a zero
/// `unitsPerEm` — which is what the byte readers answered for the empty file
/// `__canvas_fontBlob` handed them. So every reader keeps its old answer for a closed or
/// never-loaded font without a check of its own.
#[rustfmt::skip]
const FACE_READ: &str =
r#"FUNC __canvas_faceMetric(font AS Integer, word AS Integer) AS Integer
  RETURN canvas::faceAt(font, word)
END FUNC

' A glyph's advance in font units: `__canvas_glyphAdvance`'s rule, the last advance
' shared by every glyph past it.
FUNC __canvas_faceAdvance(font AS Integer, gid AS Integer) AS Integer
  LET count AS Integer = canvas::faceAt(font, __CANVAS_FACE_ADVANCES)
  IF count <= 0 THEN
    RETURN 0
  END IF
  MUT index AS Integer = gid
  IF index >= count THEN
    index = count - 1
  END IF
  RETURN canvas::faceAt(font, canvas::faceAt(font, __CANVAS_FACE_ADVANCES_AT) + index)
END FUNC

' Codepoint to glyph id: a binary search for the last run starting at or before `cp`.
FUNC __canvas_faceGlyph(font AS Integer, cp AS Integer) AS Integer
  LET at AS Integer = canvas::faceAt(font, __CANVAS_FACE_RUNS_AT)
  MUT lo AS Integer = 0
  MUT hi AS Integer = canvas::faceAt(font, __CANVAS_FACE_RUNS) - 1
  MUT found AS Integer = 0 - 1
  WHILE lo <= hi
    LET mid AS Integer = (lo + hi) / 2
    IF canvas::faceAt(font, at + mid * 3) <= cp THEN
      found = mid
      lo = mid + 1
    ELSE
      hi = mid - 1
    END IF
  END WHILE
  IF found < 0 THEN
    RETURN 0
  END IF
  IF cp > canvas::faceAt(font, at + found * 3 + 1) THEN
    RETURN 0
  END IF
  RETURN cp + canvas::faceAt(font, at + found * 3 + 2)
END FUNC"#;

/// The face index's `--debug` windows: a normal build's `__canvas_faceDump` and
/// `__canvas_faceCheck` do nothing.
#[rustfmt::skip]
const FACE_DUMP: &str =
r#"FUNC __canvas_faceDump(index AS List OF Integer) AS Nothing
END FUNC

FUNC __canvas_faceCheck(font AS Integer) AS Nothing
END FUNC"#;

/// A `--debug` build run with `MFB_CANVAS_FACE_DUMP=<file>` appends one line per loaded
/// font -- `face` and then every word of its index -- which is the only way a test can
/// see a structure the program never hands back (the `MFB_CANVAS_STATS` precedent).
/// `MFB_CANVAS_FACE_CHECK` runs `__canvas_faceCheck`, and the byte lookups it compares
/// against live here too, since nothing else calls them.
#[rustfmt::skip]
const FACE_DUMP_DEBUG: &str =
r#"FUNC __canvas_faceDump(index AS List OF Integer) AS Nothing
  LET path AS String = os::getEnvOr("MFB_CANVAS_FACE_DUMP", "")
  IF len(path) = 0 THEN
    RETURN
  END IF
  MUT line AS String = "face"
  FOR EACH word IN index
    line = line & " " & toString(word)
  NEXT
  fs::appendText(path, line & "\n") TRAP(err)
    RETURN
  END TRAP
END FUNC

' `MFB_CANVAS_FACE_CHECK=<file>`: `canvas::measureText` holds the index readers to the
' byte readers they replaced, for the font it was asked to measure -- every Unicode scalar
' through `__canvas_faceGlyph` against `__canvas_glyphIndex`, every 16-bit glyph id (and
' past it, for a format-12 font whose ids run higher) through `__canvas_faceAdvance`
' against `__canvas_glyphAdvance`, and the four metrics -- and appends the mismatch
' counts. A run-merging off-by-one shows up only on the one codepoint it moves, so
' nothing short of the whole range is a check.
FUNC __canvas_faceCheck(font AS Integer) AS Nothing
  LET path AS String = os::getEnvOr("MFB_CANVAS_FACE_CHECK", "")
  IF len(path) = 0 THEN
    RETURN
  END IF
  LET b AS List OF Byte = __canvas_fontBlob(font)
  MUT glyphs AS Integer = 0
  MUT mapped AS Integer = 0
  MUT highest AS Integer = 0
  MUT cp AS Integer = 0
  WHILE cp <= 1114111
    IF cp < 55296 OR cp > 57343 THEN
      LET want AS Integer = __canvas_glyphIndex(b, cp)
      IF __canvas_faceGlyph(font, cp) <> want THEN
        glyphs = glyphs + 1
      END IF
      IF want <> 0 THEN
        mapped = mapped + 1
      END IF
      IF want > highest THEN
        highest = want
      END IF
    END IF
    cp = cp + 1
  END WHILE
  MUT advances AS Integer = 0
  MUT gid AS Integer = 0
  LET lastGid AS Integer = __canvas_maxI(65535, highest)
  WHILE gid <= lastGid
    IF __canvas_faceAdvance(font, gid) <> __canvas_glyphAdvance(b, gid) THEN
      advances = advances + 1
    END IF
    gid = gid + 1
  END WHILE
  MUT metrics AS Integer = 0
  IF __canvas_faceMetric(font, __CANVAS_FACE_UPEM) <> __canvas_fontUnitsPerEm(b) THEN
    metrics = metrics + 1
  END IF
  IF __canvas_faceMetric(font, __CANVAS_FACE_ASCENT) <> __canvas_fontAscent(b) THEN
    metrics = metrics + 1
  END IF
  IF __canvas_faceMetric(font, __CANVAS_FACE_DESCENT) <> __canvas_fontDescent(b) THEN
    metrics = metrics + 1
  END IF
  IF __canvas_faceMetric(font, __CANVAS_FACE_LINEGAP) <> __canvas_fontLineGap(b) THEN
    metrics = metrics + 1
  END IF
  LET line AS String = "faceCheck glyphMismatches=" & toString(glyphs) & " advanceMismatches=" & toString(advances) & " metricMismatches=" & toString(metrics) & " mapped=" & toString(mapped) & " glyphIds=" & toString(lastGid + 1) & " runs=" & toString(__canvas_faceMetric(font, __CANVAS_FACE_RUNS)) & " kern=" & __canvas_faceKernText(font) & "\n"
  fs::appendText(path, line) TRAP(err)
    RETURN
  END TRAP
END FUNC

' The per-call byte lookups text read before the face index (plan-162-A), kept as the
' oracle `__canvas_faceCheck` holds the index to and nothing else: a codepoint's glyph
' through a linear scan of the chosen subtable, and a glyph's `hmtx` advance, where
' every glyph past `numberOfHMetrics` shares the last pair's advance (how monospaced and
' CJK fonts store thousands of equal widths in four bytes). A normal build has no
' caller for them, so only a `--debug` build carries them.
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
  LET groups AS Integer = __canvas_cmap12Groups(b, subtable)
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
  MUT i AS Integer = 0
  WHILE i < segs
    IF cp <= __canvas_beU16(b, endCodes + i * 2) THEN
      RETURN __canvas_cmap4Segment(b, subtable, i, cp)
    END IF
    i = i + 1
  END WHILE
  RETURN 0
END FUNC

FUNC __canvas_glyphAdvance(b AS List OF Byte, gid AS Integer) AS Integer
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

/// Text to glyphs: the **only** place a string becomes glyph ids and advances (bug-756).
///
/// `measureText`, the filled glyph run, the stroked text edges and the verify-mode
/// reference all read this one run, which is what makes "measuring and drawing use the
/// same glyph walk" (`canvas::measureText`) true by construction rather than by four
/// copies agreeing. Kerning, spacing and ligatures (plan-162) change this body and none
/// of its callers.
///
/// **Kerning** (plan-162-B) runs after every codepoint has its glyph: font by font,
/// `__canvas_kernRun` adds each pair adjustment of that font's `GPOS` `kern` feature,
/// or of its legacy `kern` table, to the advances, and only between two glyphs from the
/// same font. The adjustment stays in font units, so no caller's pixel arithmetic
/// changes, and measuring and drawing pick it up together.
///
/// `fonts` is a list of font **handles**, and list order is fallback order: each
/// codepoint takes the first font whose `cmap` maps it to a glyph other than `0`, and
/// one no font maps is glyph `0` — `.notdef` — of `fonts[0]`, exactly what a one-font
/// walk draws for it. The result is flat `(fontIndex, glyphId, advanceUnits)` triples,
/// one per codepoint, in order. Advances stay in font units: the callers scale them
/// differently on purpose (`measureText` sums Integers and scales once; the draw paths
/// accumulate scaled Floats, snapped or not), and doing it here would change pixels.
///
/// It reads each font's **face index** by handle (`__canvas_faceGlyph`,
/// `__canvas_faceAdvance`, plan-162-A) and never fetches the file. It walks font by font:
/// `fonts[0]` shapes every codepoint, `.notdef` included, and each later font replaces
/// only the triples still at glyph `0` that it maps. That is the first-font-that-maps
/// rule, and with one font it is exactly the old `cmap` + `hmtx` walk. An empty `fonts`
/// shapes nothing. A handle with no published font — a closed font — has no index, so it
/// maps nothing and is passed over: a codepoint no font maps takes glyph `0` of the
/// **first open** font, and when no font is open, glyph 0 at advance 0 of `fonts[0]`,
/// which draws nothing (plan-162-C).
#[rustfmt::skip]
const SHAPE_RUN: &str =
r#"FUNC __canvas_shapeRun(fonts AS List OF Integer, text AS String) AS List OF Integer
  LET cps AS List OF Integer = encoding::utf32Encode(text)
  LET n AS Integer = len(cps)
  ' The first open font shapes every codepoint, `.notdef` included; with none open,
  ' `fonts[0]` does, and every lookup through its missing index answers 0.
  MUT base AS Integer = 0
  MUT k AS Integer = len(fonts) - 1
  WHILE k >= 0
    IF canvas::fontRegistered(collections::getOr(fonts, k, 0)) THEN
      base = k
    END IF
    k = k - 1
  END WHILE
  MUT run AS List OF Integer = []
  LET first AS Integer = collections::getOr(fonts, base, 0)
  IF len(fonts) > 0 THEN
    MUT c AS Integer = 0
    WHILE c < n
      LET gid AS Integer = __canvas_faceGlyph(first, collections::getOr(cps, c, 0))
      run = collections::append(run, base)
      run = collections::append(run, gid)
      run = collections::append(run, __canvas_faceAdvance(first, gid))
      c = c + 1
    END WHILE
  END IF
  ' Every later font, in list order, takes the codepoints still at glyph 0 that it maps.
  ' A font before `base` is closed, and a closed one maps nothing, so neither is asked.
  MUT which AS Integer = 0
  FOR EACH f IN fonts
    IF which > base AND canvas::fontRegistered(f) THEN
      MUT c AS Integer = 0
      WHILE c < n
        IF collections::getOr(run, c * 3 + 1, 0) = 0 THEN
          LET gid AS Integer = __canvas_faceGlyph(f, collections::getOr(cps, c, 0))
          IF gid <> 0 THEN
            run = collections::set(run, c * 3, which)
            run = collections::set(run, c * 3 + 1, gid)
            run = collections::set(run, c * 3 + 2, __canvas_faceAdvance(f, gid))
          END IF
        END IF
        c = c + 1
      END WHILE
    END IF
    which = which + 1
  NEXT
  which = 0
  FOR EACH f IN fonts
    run = __canvas_kernRun(f, which, run)
    which = which + 1
  NEXT
  RETURN run
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_shapeRun", SHAPE_RUN));
    pkg.add_helper(RegistryHelper::always("canvas_fontBytes", FONT_BYTES_READ));
    pkg.add_helper(RegistryHelper::always("canvas_fontTable", FONT_TABLE));
    pkg.add_helper(RegistryHelper::always("canvas_fontMetrics", FONT_METRICS));
    pkg.add_helper(RegistryHelper::always("canvas_fontCmap", FONT_CMAP));
    pkg.add_helper(RegistryHelper::always("canvas_faceIndex", FACE_INDEX));
    pkg.add_helper(RegistryHelper::always("canvas_faceRead", FACE_READ));
    for helper in RegistryHelper::debug_split("canvas_faceDump", FACE_DUMP, FACE_DUMP_DEBUG) {
        pkg.add_helper(helper);
    }
}
