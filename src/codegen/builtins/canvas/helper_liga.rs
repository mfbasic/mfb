//! Standard ligatures (plan-162-F): the `GSUB` `liga` feature's ligature substitutions
//! and the `GDEF` ligature carets, read once at load into the face index, and applied by
//! `__canvas_shapeRun`.
//!
//! **What is read.** `GSUB` lookups of type 4 (LigatureSubst, format 1), directly or
//! through type 7 (Extension), under the `liga` feature, found by the same script and
//! LangSys walk as kerning (`helper_kern.rs:__canvas_layoutLookups`, given the `GSUB`
//! table). No other feature (`clig`, `dlig`, `calt`, `rlig`) and no other lookup type is
//! applied. `GDEF`'s LigCaretList gives, per ligature glyph, the x of each boundary
//! between its components; a caret of format 2 (a contour point) is not resolved, and a
//! ligature with one has no carets — its boundaries are divided evenly instead.
//!
//! **The sections.** Header word 14 of the face index (layout version 4) is the ligature
//! section, word 15 the caret section; 0 means none. Offsets count from each section's
//! own first word:
//!
//! | Block | Words |
//! |---|---|
//! | ligature section | `lookupCount`, one offset per lookup |
//! | lookup | `subtableCount`, one offset per subtable |
//! | subtable | `C, C × (firstGid, lastGid, coverageIndex), S, S + 1 set starts, ligatures` |
//! | a ligature | `ligatureGid, componentCount, componentCount − 1 component gids` |
//! | caret section | `C, C × coverage runs, N, N + 1 starts, coordinates` |
//!
//! A set's ligatures are the words from its start to the next set's, in the font's order:
//! the first that matches wins, which is also the longest, because fonts list longer
//! ligatures first. Every file-supplied count is clipped to the file, as kerning's are.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Building the two sections at load.
#[rustfmt::skip]
const LIGA_SECTION: &str =
r#"' The ligature section for font bytes `b`, or [] for a font with no `liga`.
FUNC __canvas_ligaSection(b AS List OF Byte) AS List OF Integer
  LET gsub AS Integer = __canvas_fontTable(b, "GSUB")
  IF gsub < 0 THEN
    RETURN []
  END IF
  LET lookups AS List OF Integer = __canvas_layoutLookups(b, gsub, "liga")
  IF len(lookups) = 0 THEN
    RETURN []
  END IF
  LET list AS Integer = gsub + __canvas_beU16(b, gsub + 8)
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
        IF kind = 7 AND __canvas_beU16(b, st) = 1 THEN
          k = __canvas_beU16(b, st + 2)
          st = st + __canvas_beU32(b, st + 4)
        END IF
        IF k = 4 AND __canvas_beU16(b, st) = 1 THEN
          parts = collections::append(parts, __canvas_ligatureSubst(b, st))
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

FUNC __canvas_ligatureSubst(b AS List OF Byte, st AS Integer) AS List OF Integer
  LET covAt AS Integer = __canvas_beU16(b, st + 2)
  MUT out AS List OF Integer = __canvas_coverageRuns(b, st + covAt, covAt <> 0)
  LET sets AS Integer = __canvas_clipCount(b, st + 6, __canvas_beU16(b, st + 4), 2)
  MUT starts AS List OF Integer = [0]
  MUT ligs AS List OF Integer = []
  MUT k AS Integer = 0
  WHILE k < sets
    LET set AS Integer = st + __canvas_beU16(b, st + 6 + k * 2)
    LET n AS Integer = __canvas_clipCount(b, set + 2, __canvas_beU16(b, set), 2)
    MUT j AS Integer = 0
    WHILE j < n
      LET lig AS Integer = set + __canvas_beU16(b, set + 2 + j * 2)
      LET components AS Integer = __canvas_beU16(b, lig + 2)
      ' A ligature of fewer than two components substitutes nothing, and one whose
      ' components run past the file is not one this file can hold.
      IF components >= 2 AND lig + 4 + (components - 1) * 2 <= len(b) THEN
        ligs = collections::append(ligs, __canvas_beU16(b, lig))
        ligs = collections::append(ligs, components)
        MUT c AS Integer = 1
        WHILE c < components
          ligs = collections::append(ligs, __canvas_beU16(b, lig + 4 + (c - 1) * 2))
          c = c + 1
        END WHILE
      END IF
      j = j + 1
    END WHILE
    starts = collections::append(starts, len(ligs))
    k = k + 1
  END WHILE
  out = collections::append(out, sets)
  out = collections::append(out, starts)
  out = collections::append(out, ligs)
  RETURN out
END FUNC

' The caret section for font bytes `b`, or [] for a font with no `GDEF` LigCaretList:
' per covered ligature glyph, the x of each component boundary in font units. A
' ligature with a contour-point caret (format 2) has none.
FUNC __canvas_caretSection(b AS List OF Byte) AS List OF Integer
  LET gdef AS Integer = __canvas_fontTable(b, "GDEF")
  IF gdef < 0 THEN
    RETURN []
  END IF
  LET listAt AS Integer = __canvas_beU16(b, gdef + 8)
  IF listAt = 0 THEN
    RETURN []
  END IF
  LET list AS Integer = gdef + listAt
  LET covAt AS Integer = __canvas_beU16(b, list)
  MUT out AS List OF Integer = __canvas_coverageRuns(b, list + covAt, covAt <> 0)
  LET glyphs AS Integer = __canvas_clipCount(b, list + 4, __canvas_beU16(b, list + 2), 2)
  MUT starts AS List OF Integer = [0]
  MUT carets AS List OF Integer = []
  MUT g AS Integer = 0
  WHILE g < glyphs
    LET lig AS Integer = list + __canvas_beU16(b, list + 4 + g * 2)
    LET n AS Integer = __canvas_clipCount(b, lig + 2, __canvas_beU16(b, lig), 2)
    MUT these AS List OF Integer = []
    MUT usable AS Boolean = TRUE
    MUT c AS Integer = 0
    WHILE c < n
      LET value AS Integer = lig + __canvas_beU16(b, lig + 2 + c * 2)
      LET format AS Integer = __canvas_beU16(b, value)
      IF format = 1 OR format = 3 THEN
        these = collections::append(these, __canvas_beS16(b, value + 2))
      ELSE
        usable = FALSE
      END IF
      c = c + 1
    END WHILE
    IF usable THEN
      carets = collections::append(carets, these)
    END IF
    starts = collections::append(starts, len(carets))
    g = g + 1
  END WHILE
  IF glyphs = 0 THEN
    RETURN []
  END IF
  out = collections::append(out, glyphs)
  out = collections::append(out, starts)
  out = collections::append(out, carets)
  RETURN out
END FUNC"#;

/// Reading the sections by font handle, and substituting.
#[rustfmt::skip]
const LIGA_READ: &str =
r#"' Apply font `which`'s ligatures to the shaped `run` (before kerning), lookup by
' lookup, left to right over the glyphs both from that font: at each, the first subtable
' covering the glyph, and in it the first ligature whose components follow, replaces
' them with the ligature glyph. That glyph keeps the first component's scalar index, and
' its component count is the sum of what it replaced. A run that came through with
' letter-spacing never gets here (`__canvas_shapeRun`).
FUNC __canvas_ligaRun(font AS Integer, which AS Integer, run AS List OF Integer) AS List OF Integer
  LET section AS Integer = canvas::faceAt(font, __CANVAS_FACE_LIGA)
  IF section <= 0 THEN
    RETURN run
  END IF
  MUT current AS List OF Integer = run
  LET lookups AS Integer = canvas::faceAt(font, section)
  MUT l AS Integer = 0
  WHILE l < lookups
    LET lk AS Integer = section + canvas::faceAt(font, section + 1 + l)
    LET subs AS Integer = canvas::faceAt(font, lk)
    LET glyphs AS Integer = len(current) / __CANVAS_RUN_STRIDE
    MUT out AS List OF Integer = []
    MUT i AS Integer = 0
    WHILE i < glyphs
      LET at AS Integer = i * __CANVAS_RUN_STRIDE
      MUT taken AS Integer = 0
      IF collections::getOr(current, at + __CANVAS_RUN_FONT, 0 - 1) = which THEN
        MUT s AS Integer = 0
        WHILE s < subs AND taken = 0
          taken = __canvas_ligaMatch(font, lk + canvas::faceAt(font, lk + 1 + s), which, current, i)
          s = s + 1
        END WHILE
      END IF
      IF taken > 0 THEN
        MUT components AS Integer = 0
        MUT c AS Integer = 0
        WHILE c < taken
          components = components + collections::getOr(current, (i + c) * __CANVAS_RUN_STRIDE + __CANVAS_RUN_COMPONENTS, 1)
          c = c + 1
        END WHILE
        out = collections::append(out, which)
        out = collections::append(out, __CANVAS_LIGA_GLYPH)
        out = collections::append(out, __canvas_faceAdvance(font, __CANVAS_LIGA_GLYPH))
        out = collections::append(out, collections::getOr(current, at + __CANVAS_RUN_SCALAR, 0))
        out = collections::append(out, 0)
        out = collections::append(out, components)
        i = i + taken
      ELSE
        MUT w AS Integer = 0
        WHILE w < __CANVAS_RUN_STRIDE
          out = collections::append(out, collections::getOr(current, at + w, 0))
          w = w + 1
        END WHILE
        i = i + 1
      END IF
    END WHILE
    current = out
    l = l + 1
  END WHILE
  RETURN current
END FUNC

MUT __CANVAS_LIGA_GLYPH AS Integer = 0

' Does subtable `st` form a ligature at glyph `i` of `run`? Returns how many glyphs the
' ligature takes (0 for none) and leaves its glyph in __CANVAS_LIGA_GLYPH.
FUNC __canvas_ligaMatch(font AS Integer, st AS Integer, which AS Integer, run AS List OF Integer, i AS Integer) AS Integer
  LET covCount AS Integer = canvas::faceAt(font, st)
  LET first AS Integer = collections::getOr(run, i * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GLYPH, 0)
  LET r AS Integer = __canvas_faceRunAt(font, st + 1, covCount, first)
  IF r < 0 THEN
    RETURN 0
  END IF
  LET index AS Integer = canvas::faceAt(font, st + 1 + r * 3 + 2) + first - canvas::faceAt(font, st + 1 + r * 3)
  LET p AS Integer = st + 1 + covCount * 3
  LET sets AS Integer = canvas::faceAt(font, p)
  IF index >= sets THEN
    RETURN 0
  END IF
  LET ligs AS Integer = p + 2 + sets
  MUT at AS Integer = ligs + canvas::faceAt(font, p + 1 + index)
  LET stop AS Integer = ligs + canvas::faceAt(font, p + 2 + index)
  LET glyphs AS Integer = len(run) / __CANVAS_RUN_STRIDE
  WHILE at < stop
    LET components AS Integer = canvas::faceAt(font, at + 1)
    MUT ok AS Boolean = i + components <= glyphs
    MUT c AS Integer = 1
    WHILE ok AND c < components
      LET g AS Integer = (i + c) * __CANVAS_RUN_STRIDE
      IF collections::getOr(run, g + __CANVAS_RUN_FONT, 0 - 1) <> which OR collections::getOr(run, g + __CANVAS_RUN_GLYPH, 0 - 1) <> canvas::faceAt(font, at + 1 + c) THEN
        ok = FALSE
      END IF
      c = c + 1
    END WHILE
    IF ok THEN
      __CANVAS_LIGA_GLYPH = canvas::faceAt(font, at)
      RETURN components
    END IF
    at = at + 1 + components
  END WHILE
  RETURN 0
END FUNC

' The x of boundary `k` (1-based, between components k and k + 1) of ligature glyph `gid`
' in font units, from the font's `GDEF` carets, or __CANVAS_FACE_UNSET when it has none.
FUNC __canvas_faceCaret(font AS Integer, gid AS Integer, k AS Integer) AS Integer
  LET section AS Integer = canvas::faceAt(font, __CANVAS_FACE_CARETS)
  IF section <= 0 THEN
    RETURN __CANVAS_FACE_UNSET
  END IF
  LET covCount AS Integer = canvas::faceAt(font, section)
  LET r AS Integer = __canvas_faceRunAt(font, section + 1, covCount, gid)
  IF r < 0 THEN
    RETURN __CANVAS_FACE_UNSET
  END IF
  LET index AS Integer = canvas::faceAt(font, section + 1 + r * 3 + 2) + gid - canvas::faceAt(font, section + 1 + r * 3)
  LET p AS Integer = section + 1 + covCount * 3
  LET glyphs AS Integer = canvas::faceAt(font, p)
  IF index >= glyphs THEN
    RETURN __CANVAS_FACE_UNSET
  END IF
  LET start AS Integer = canvas::faceAt(font, p + 1 + index)
  LET stop AS Integer = canvas::faceAt(font, p + 2 + index)
  IF k < 1 OR start + k - 1 >= stop THEN
    RETURN __CANVAS_FACE_UNSET
  END IF
  RETURN canvas::faceAt(font, p + 2 + glyphs + start + k - 1)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_ligaSection", LIGA_SECTION));
    pkg.add_helper(RegistryHelper::always("canvas_ligaRead", LIGA_READ));
}
