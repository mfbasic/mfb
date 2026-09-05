//! The glyph coverage cache: rasterise each `(font, size, glyph)` once, then blit it.
//!
//! **This is what makes text usable, and Correction 12 is why it exists.** Drawing a
//! string as one polygon is correct — a glyph is closed contours and
//! `__canvas_edgeDistance` turns closed contours into a signed distance — but that
//! evaluation is `O(edges)` *per pixel*, and a curved glyph costs about 160 flattened
//! edges. Twelve characters at size 120 is 1899 edges over an 800x150 box: ~228 million
//! segment-distance evaluations, and **8.1 seconds** for one measured frame.
//!
//! The fix is not a faster distance function, it is doing the work once. A glyph's
//! coverage depends only on its outline and its size, never on where it is drawn or how
//! many times, so `(font, size, glyph)` is rasterised into a byte per pixel and every
//! later use is a blend of that byte. Twelve characters becomes twelve blits.
//!
//! **It is a memoisation, not a different renderer.** Each byte is
//! `__canvas_coverage(__canvas_edgeDistance(...))` — the same call the polygon path
//! makes at the same pixel centres — so a cached glyph and a rasterised one produce the
//! same pixels, not merely similar ones. The two do differ where glyph boxes *overlap*:
//! the polygon path takes the nearest edge across the whole string, while blitting
//! blends each glyph in turn. Ordinary text does not overlap, and a stroked run takes
//! the polygon path anyway (see below), so the difference has no reachable case today.
//!
//! **Stroked text keeps the polygon path.** A stroke needs `abs(distance) - half`, so it
//! needs the signed distance rather than the clamped coverage, and caching a distance
//! accurately enough for antialiasing would cost four bytes a pixel instead of one. The
//! geometry builder emits the cheap kind only for fill-only text, which is what text
//! almost always is, and stroked text stays correct at the old cost.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The cache's parallel arrays, mirroring the geometry cache's shape.
///
/// Coverage bytes all live in one growing `List OF Byte` and an entry records where its
/// own run starts, for the reason `__CANVAS_GEO_DATA` does the same: a list of lists
/// would copy a whole glyph's bytes on every append into the outer list.
///
/// The key packs the font handle, the size in sixteenths of a pixel, and the glyph id.
/// Size is quantised because a caller animating a size would otherwise fill the cache
/// with entries it uses once, and a sixteenth of a pixel is below what the coverage ramp
/// can express anyway.
#[rustfmt::skip]
const GLYPH_CACHE_STATE: &str =
r#"MUT __CANVAS_GLYPH_KEYS AS List OF Integer = []
MUT __CANVAS_GLYPH_META AS List OF Integer = []
MUT __CANVAS_GLYPH_COV AS List OF Byte = []
MUT __CANVAS_GLYPH_LASTUSED AS List OF Integer = []
MUT __CANVAS_GLYPH_REV AS Integer = 0
MUT __CANVAS_GLYPH_EVICTIONS AS Integer = 0
MUT __CANVAS_GLYPH_BUDGET AS Integer = 0
MUT __CANVAS_GLYPH_NEXTEVICT AS Integer = 0

' The entries of the glyph run currently being built, if any. A run is not in the
' geometry cache until it is finished, so this is the only way an eviction pass
' triggered part-way through one can see the glyphs that run has already claimed.
MUT __CANVAS_GLYPH_PINS AS List OF Integer = []

' The coverage budget, in bytes. A megabyte is a few hundred glyphs at body sizes and
' a couple of dozen at display sizes -- enough that ordinary text never evicts, and
' small enough that a program animating a size cannot grow the cache without bound.
' Crossing it is not a failure: it is the point at which the cache starts behaving like
' a cache.
'
' `MFB_CANVAS_GLYPH_BUDGET` shrinks it. Without that, a test cannot force an eviction
' at a cost it can afford: filling a megabyte with the four-edge fixture glyph takes a
' scene far larger than one that also has to be checked pixel by pixel, so the eviction
' path would go untested while the test that was meant to cover it passed. Resolved
' once and cached, so the ordinary path is a compare against a global rather than a
' `getenv` per glyph.
FUNC __canvas_budgetOr(text AS String, fallback AS Integer) AS Integer
  IF len(text) = 0 THEN
    RETURN fallback
  END IF
  RETURN toInt(text)
  TRAP(err)
    RETURN fallback
  END TRAP
END FUNC

FUNC __canvas_glyphBudget() AS Integer
  IF __CANVAS_GLYPH_BUDGET = 0 THEN
    __CANVAS_GLYPH_BUDGET = __canvas_budgetOr(os::getEnvOr("MFB_CANVAS_GLYPH_BUDGET", ""), 1048576)
  END IF
  RETURN __CANVAS_GLYPH_BUDGET
END FUNC

FUNC __canvas_glyphKey(fontId AS Integer, sizeQ AS Integer, gid AS Integer) AS Integer
  ' Every term is folded into a fixed width before it is combined. `fontId` is the
  ' resource record's **address**, so the textbook `id * prime` overflows an `Integer`
  ' on the first multiply -- and the raise lands on the graphics thread, which has no
  ' handler, so the thread dies and the worker waits for a frame that never comes. A
  ' hang with a thread missing, not an error message (Correction 13).
  '
  ' The low three bits of an 8-aligned address carry nothing, so shifting them out
  ' before folding to 24 bits keeps more of what distinguishes two fonts.
  LET f AS Integer = (fontId / 8) MOD 16777216
  RETURN ((f * 65536) + (sizeQ MOD 65536)) * 65536 + (gid MOD 65536)
END FUNC

FUNC __canvas_sizeQ(size AS Float) AS Integer
  RETURN toInt(size * 16.0 + 0.5)
END FUNC"#;

/// Rasterise a glyph at the origin and return its cache entry index.
///
/// The bitmap is laid out relative to the glyph's own ink box, and the entry records
/// that box's offset from the pen so the blit can put it back. Rasterising at the origin
/// rather than at the pen is the whole point: the same glyph at a different position is
/// the same bitmap, which is what makes a repeated character free.
///
/// Sub-pixel positioning is deliberately not modelled. The pen is rounded to a whole
/// pixel at blit time, so an `x` of 10.3 and one of 10.7 share a bitmap. That is
/// visible as slightly uneven spacing in a proportional font and is the standard
/// trade — the alternative is one bitmap per sub-pixel phase, which multiplies the cache
/// by the number of phases for a difference smaller than the hinting this build does not
/// do either.
#[rustfmt::skip]
const GLYPH_CACHE_BUILD: &str =
r#"FUNC __canvas_glyphEntry(b AS List OF Byte, fontId AS Integer, gid AS Integer, size AS Float, scale AS Float) AS Integer
  LET sizeQ AS Integer = __canvas_sizeQ(size)
  LET key AS Integer = __canvas_glyphKey(fontId, sizeQ, gid)
  MUT i AS Integer = 0
  LET known AS Integer = len(__CANVAS_GLYPH_KEYS)
  __CANVAS_GLYPH_REV = __CANVAS_GLYPH_REV + 1
  WHILE i < known
    IF collections::getOr(__CANVAS_GLYPH_KEYS, i, 0) = key THEN
      __CANVAS_GLYPH_LASTUSED = collections::set(__CANVAS_GLYPH_LASTUSED, i, __CANVAS_GLYPH_REV)
      RETURN i
    END IF
    i = i + 1
  END WHILE

  ' Over budget is necessary but not sufficient. A pass can be entitled to free
  ' nothing -- pinning is absolute, and a scene of 256 distinct items can pin the whole
  ' cache -- and re-running a full compaction on every subsequent insert would then be
  ' quadratic in the cache size for a scene we are required to keep whole. So a pass
  ' that frees little defers the next one until the cache has grown by another half
  ' budget. Memory stays bounded because the pins are: the geometry cache is capped at
  ' `__CANVAS_GEO_CAPACITY` items, and a glyph unpins as soon as the item referencing
  ' it is evicted from there.
  IF len(__CANVAS_GLYPH_COV) > __canvas_glyphBudget() AND len(__CANVAS_GLYPH_COV) >= __CANVAS_GLYPH_NEXTEVICT THEN
    __canvas_glyphEvict()
    __CANVAS_GLYPH_NEXTEVICT = len(__CANVAS_GLYPH_COV) + __canvas_glyphBudget() / 2
  END IF

  ' Flatten at the origin: the pen offset is applied when the bitmap is blitted.
  LET edges AS List OF Float = __canvas_glyphEdges(b, gid, scale, 0.0, 0.0, [])
  LET count AS Integer = len(edges) / 5
  MUT x0 AS Integer = 0
  MUT y0 AS Integer = 0
  MUT w AS Integer = 0
  MUT h AS Integer = 0
  MUT start AS Integer = len(__CANVAS_GLYPH_COV)
  IF count > 0 THEN
    MUT minX AS Float = collections::getOr(edges, 0, 0.0)
    MUT maxX AS Float = minX
    MUT minY AS Float = collections::getOr(edges, 1, 0.0)
    MUT maxY AS Float = minY
    MUT e AS Integer = 0
    WHILE e < len(edges)
      LET ex0 AS Float = collections::getOr(edges, e, 0.0)
      LET ey0 AS Float = collections::getOr(edges, e + 1, 0.0)
      LET ex1 AS Float = ex0 + collections::getOr(edges, e + 2, 0.0)
      LET ey1 AS Float = ey0 + collections::getOr(edges, e + 3, 0.0)
      minX = __canvas_minF(minX, __canvas_minF(ex0, ex1))
      maxX = __canvas_maxF(maxX, __canvas_maxF(ex0, ex1))
      minY = __canvas_minF(minY, __canvas_minF(ey0, ey1))
      maxY = __canvas_maxF(maxY, __canvas_maxF(ey0, ey1))
      e = e + 5
    END WHILE
    ' One pixel of margin on each side, so the antialiased rim of a shape that ends
    ' exactly on a pixel boundary is inside the bitmap rather than clipped by it.
    x0 = toInt(__canvas_floorF(minX)) - 1
    y0 = toInt(__canvas_floorF(minY)) - 1
    w = toInt(__canvas_floorF(maxX)) + 2 - x0
    h = toInt(__canvas_floorF(maxY)) + 2 - y0
    ' The bitmap is sized by the outline, and the outline is the file's: int16
    ' coordinates over the smallest legal `unitsPerEm` put one glyph 375,000 px a side
    ' at size 200 (bug-509, DEC-53). 8192 a side and 2^24 bytes -- sixteen times the
    ' whole cache budget, and larger than any glyph a 4K display shows whole -- is the
    ' most one entry may cost. Past it the entry is recorded empty and the glyph draws
    ' nothing. It cannot raise: this runs on the graphics thread, where a raise is a
    ' hang (Correction 13). The sides are checked before their product for the same
    ' overflow reason as the image caps.
    IF w > 8192 OR h > 8192 THEN
      w = 0
      h = 0
    ELSEIF w * h > 16777216 THEN
      w = 0
      h = 0
    END IF
    MUT cov AS List OF Byte = __CANVAS_GLYPH_COV
    MUT row AS Integer = 0
    WHILE row < h
      LET py AS Float = toFloat(y0 + row) + 0.5
      MUT col AS Integer = 0
      WHILE col < w
        LET px AS Float = toFloat(x0 + col) + 0.5
        cov = collections::append(cov, toByte(__canvas_coverage(__canvas_edgeDistanceIn(edges, count, px, py))))
        col = col + 1
      END WHILE
      row = row + 1
    END WHILE
    __CANVAS_GLYPH_COV = cov
  END IF

  ' The new entry's index is read HERE, not from the `known` the miss scan used. An
  ' eviction pass runs between the two and renumbers everything that survived it, so
  ' `known` is stale by exactly the number of entries that pass dropped -- and the run
  ' would then carry an index to an entry that does not exist, which the blit reads as
  ' a zero-sized bitmap and draws as nothing. Six of a 300-glyph scene's items vanished
  ' that way, silently, with the cache reporting a healthy hit rate.
  LET index AS Integer = len(__CANVAS_GLYPH_KEYS)
  __CANVAS_GLYPH_KEYS = collections::append(__CANVAS_GLYPH_KEYS, key)
  __CANVAS_GLYPH_LASTUSED = collections::append(__CANVAS_GLYPH_LASTUSED, __CANVAS_GLYPH_REV)
  MUT meta AS List OF Integer = __CANVAS_GLYPH_META
  meta = collections::append(meta, x0)
  meta = collections::append(meta, y0)
  meta = collections::append(meta, w)
  meta = collections::append(meta, h)
  meta = collections::append(meta, start)
  __CANVAS_GLYPH_META = meta
  RETURN index
END FUNC"#;

/// The same nearest-edge-plus-crossing-count query `__canvas_edgeDistance` runs, over a
/// caller-held edge list rather than over `__CANVAS_GEO_DATA`.
///
/// It exists because a glyph is rasterised **before** it is in the geometry cache — the
/// cache stores the finished bitmap, not the outline — so the query needs to read the
/// edges from a local. Keeping the arithmetic identical to `__canvas_edgeDistance` is
/// what makes a cached glyph and a rasterised one the same pixels rather than merely
/// similar ones; if one of them ever changes, the other has to change with it.
#[rustfmt::skip]
const GLYPH_DISTANCE: &str =
r#"FUNC __canvas_edgeDistanceIn(edges AS List OF Float, count AS Integer, px AS Float, py AS Float) AS Float
  MUT best AS Float = 1000000.0
  MUT inside AS Boolean = FALSE
  MUT e AS Integer = 0
  WHILE e < count
    LET base AS Integer = e * 5
    LET ax AS Float = collections::getOr(edges, base, 0.0)
    LET ay AS Float = collections::getOr(edges, base + 1, 0.0)
    LET dx AS Float = collections::getOr(edges, base + 2, 0.0)
    LET dy AS Float = collections::getOr(edges, base + 3, 0.0)
    LET invLenSq AS Float = collections::getOr(edges, base + 4, 0.0)
    LET wx AS Float = px - ax
    LET wy AS Float = py - ay
    LET t AS Float = __canvas_minF(__canvas_maxF((wx * dx + wy * dy) * invLenSq, 0.0), 1.0)
    LET qx AS Float = wx - t * dx
    LET qy AS Float = wy - t * dy
    best = __canvas_minF(best, math::sqrt(qx * qx + qy * qy))
    LET by AS Float = ay + dy
    IF (ay > py) <> (by > py) THEN
      LET u AS Float = (py - ay) / dy
      IF px < ax + u * dx THEN
        inside = NOT inside
      END IF
    END IF
    e = e + 1
  END WHILE
  IF inside THEN
    RETURN 0.0 - best
  END IF
  RETURN best
END FUNC

FUNC __canvas_floorF(v AS Float) AS Float
  LET t AS Float = toFloat(toInt(v))
  IF t > v THEN
    RETURN t - 1.0
  END IF
  RETURN t
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_glyphCacheState",
        GLYPH_CACHE_STATE,
    ));
    pkg.add_helper(RegistryHelper::always(
        "canvas_glyphDistance",
        GLYPH_DISTANCE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_glyphEvict", GLYPH_EVICT));
    pkg.add_helper(RegistryHelper::always(
        "canvas_glyphCacheBuild",
        GLYPH_CACHE_BUILD,
    ));
}

/// Drop the least-recently-used glyphs, keeping every one a live scene still draws.
///
/// **Pinning is by reference, not by a flag.** A glyph the scene draws is not "recently
/// used" in the obvious sense: once a text item is in the geometry cache, drawing it
/// blits by index and never asks for the glyph again, so its last-used revision stops
/// advancing the moment it becomes cheap. Marking pins from the geometry cache's own
/// `__CANVAS_GEO_TEXT` entries is therefore the only definition that is actually true —
/// a last-used cutoff would evict exactly the glyphs on screen.
///
/// Surviving entries are **renumbered**, and the geometry cache's runs are rewritten to
/// match, rather than the geometry cache being cleared. Clearing it would be simpler and
/// would re-flatten every string on screen at the moment the program is already under
/// memory pressure — the worst possible time to add work.
///
/// Eviction is rare by construction, so a full compaction pass is the right shape: it
/// runs when the cache crosses its budget, not per glyph or per frame.
#[rustfmt::skip]
const GLYPH_EVICT: &str =
r#"SUB __canvas_glyphEvict()
  LET count AS Integer = len(__CANVAS_GLYPH_KEYS)
  IF count = 0 THEN
    EXIT SUB
  END IF
  __CANVAS_GLYPH_EVICTIONS = __CANVAS_GLYPH_EVICTIONS + 1

  ' Pin every entry a live geometry record references.
  MUT pinned AS List OF Boolean = []
  MUT p AS Integer = 0
  WHILE p < count
    pinned = collections::append(pinned, FALSE)
    p = p + 1
  END WHILE
  ' The offsets to walk: the geometry cache's own, plus any the frame in progress is
  ' holding that the cache has already evicted. That second set is not an edge case -- a
  ' scene larger than `__CANVAS_GEO_CAPACITY` produces it on every frame, and the glyphs
  ' it names are about to be drawn.
  MUT live AS List OF Integer = []
  FOR EACH offset IN __CANVAS_GEO_OFFSETS
    live = collections::append(live, offset)
  NEXT
  LET cached AS Integer = len(live)
  FOR EACH offset IN __CANVAS_GEO_LIVE
    MUT seen AS Boolean = FALSE
    MUT c AS Integer = 0
    WHILE c < cached
      IF collections::getOr(live, c, 0 - 1) = offset THEN
        seen = TRUE
      END IF
      c = c + 1
    END WHILE
    IF NOT seen THEN
      ' Deduped, because the remap below rewrites each run exactly once. Twice would
      ' apply the map to its own output and produce an index that names some other
      ' glyph -- a wrong picture rather than a missing one.
      live = collections::append(live, offset)
    END IF
  NEXT

  MUT slot AS Integer = 0
  LET slots AS Integer = len(live)
  WHILE slot < slots
    LET offset AS Integer = collections::getOr(live, slot, 0)
    IF toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0)) = __CANVAS_GEO_TEXT THEN
      LET glyphs AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
      MUT g AS Integer = 0
      WHILE g < glyphs
        LET entry AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + g * 3, 0.0))
        IF entry >= 0 AND entry < count THEN
          pinned = collections::set(pinned, entry, TRUE)
        END IF
        g = g + 1
      END WHILE
    END IF
    slot = slot + 1
  END WHILE
  MUT q AS Integer = 0
  LET building AS Integer = len(__CANVAS_GLYPH_PINS)
  WHILE q < building
    LET entry AS Integer = collections::getOr(__CANVAS_GLYPH_PINS, q, 0 - 1)
    IF entry >= 0 AND entry < count THEN
      pinned = collections::set(pinned, entry, TRUE)
    END IF
    q = q + 1
  END WHILE

  ' Keep a pinned entry always, and an unpinned one only if it is newer than the
  ' median revision -- which halves the cache in one pass rather than evicting one
  ' glyph at a time and re-entering here on the next insert.
  MUT newest AS Integer = 0
  MUT oldest AS Integer = __CANVAS_GLYPH_REV
  MUT i AS Integer = 0
  WHILE i < count
    LET used AS Integer = collections::getOr(__CANVAS_GLYPH_LASTUSED, i, 0)
    IF used > newest THEN
      newest = used
    END IF
    IF used < oldest THEN
      oldest = used
    END IF
    i = i + 1
  END WHILE
  LET cutoff AS Integer = (oldest + newest) / 2

  MUT remap AS List OF Integer = []
  MUT keys AS List OF Integer = []
  MUT meta AS List OF Integer = []
  MUT used2 AS List OF Integer = []
  MUT cov AS List OF Byte = []
  MUT kept AS Integer = 0
  i = 0
  WHILE i < count
    LET keep AS Boolean = collections::getOr(pinned, i, FALSE) OR collections::getOr(__CANVAS_GLYPH_LASTUSED, i, 0) >= cutoff
    IF keep THEN
      LET base AS Integer = i * 5
      LET w AS Integer = collections::getOr(__CANVAS_GLYPH_META, base + 2, 0)
      LET h AS Integer = collections::getOr(__CANVAS_GLYPH_META, base + 3, 0)
      LET from AS Integer = collections::getOr(__CANVAS_GLYPH_META, base + 4, 0)
      remap = collections::append(remap, kept)
      keys = collections::append(keys, collections::getOr(__CANVAS_GLYPH_KEYS, i, 0))
      used2 = collections::append(used2, collections::getOr(__CANVAS_GLYPH_LASTUSED, i, 0))
      meta = collections::append(meta, collections::getOr(__CANVAS_GLYPH_META, base, 0))
      meta = collections::append(meta, collections::getOr(__CANVAS_GLYPH_META, base + 1, 0))
      meta = collections::append(meta, w)
      meta = collections::append(meta, h)
      meta = collections::append(meta, len(cov))
      MUT b AS Integer = 0
      WHILE b < w * h
        cov = collections::append(cov, collections::getOr(__CANVAS_GLYPH_COV, from + b, toByte(0)))
        b = b + 1
      END WHILE
      kept = kept + 1
    ELSE
      remap = collections::append(remap, 0 - 1)
    END IF
    i = i + 1
  END WHILE

  __CANVAS_GLYPH_KEYS = keys
  __CANVAS_GLYPH_META = meta
  __CANVAS_GLYPH_LASTUSED = used2
  __CANVAS_GLYPH_COV = cov

  ' Rewrite the runs so every index still names the glyph it named before.
  MUT data AS List OF Float = __CANVAS_GEO_DATA
  slot = 0
  WHILE slot < slots
    LET offset AS Integer = collections::getOr(live, slot, 0)
    IF toInt(collections::getOr(data, offset, 0.0)) = __CANVAS_GEO_TEXT THEN
      LET glyphs AS Integer = toInt(collections::getOr(data, offset + 20, 0.0))
      MUT g AS Integer = 0
      WHILE g < glyphs
        LET at AS Integer = offset + __CANVAS_GEO_HEADER + g * 3
        LET entry AS Integer = toInt(collections::getOr(data, at, 0.0))
        IF entry >= 0 AND entry < count THEN
          data = collections::set(data, at, toFloat(collections::getOr(remap, entry, 0 - 1)))
        END IF
        g = g + 1
      END WHILE
    END IF
    slot = slot + 1
  END WHILE
  __CANVAS_GEO_DATA = data

  ' And the run under construction, for the same reason and by the same map.
  MUT pins AS List OF Integer = []
  q = 0
  WHILE q < building
    LET entry AS Integer = collections::getOr(__CANVAS_GLYPH_PINS, q, 0 - 1)
    IF entry >= 0 AND entry < count THEN
      pins = collections::append(pins, collections::getOr(remap, entry, 0 - 1))
    ELSE
      pins = collections::append(pins, entry)
    END IF
    q = q + 1
  END WHILE
  __CANVAS_GLYPH_PINS = pins
END SUB"#;
