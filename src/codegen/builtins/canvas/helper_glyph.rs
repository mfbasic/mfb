//! Glyph outlines: `loca` → `glyf` → quadratic contours → the flat edge list the
//! polygon rasteriser already consumes.
//!
//! **Text is a polygon.** That is the payoff of hand-rolling the reader
//! (`.ai/canvas-threading.md` §12): a glyph is a set of closed contours, and
//! `__canvas_edgeDistance` already turns closed contours into a signed distance. So a
//! `Text` item produces a `__CANVAS_GEO_POLYGON` header with every glyph's edges in its
//! tail, and the renderer — software, Metal and Vulkan alike — needs no new arm at all.
//! Fill, stroke, antialiasing and blending come along unchanged, which is also what
//! keeps a glyph's edge pixels consistent with a circle's.
//!
//! The fill rule is the one seam worth naming. TrueType specifies **non-zero winding**
//! and `__canvas_edgeDistance` counts **even-odd** crossings. They agree on every glyph
//! whose counters are wound opposite their outer contour, which is what the format
//! requires and what every well-made font does; they differ only where two contours of
//! one glyph *overlap*, which good fonts avoid because it renders badly everywhere.
//! Adopting even-odd is therefore a real if narrow limitation, and it is the price of
//! text sharing one rasteriser with every other primitive rather than having its own.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// `loca` → the glyph's byte range inside `glyf`.
///
/// `indexToLocFormat` in `head` picks the width: `0` is 16-bit offsets stored halved,
/// `1` is 32-bit stored as-is. Getting that wrong does not fail, it reads a different
/// glyph — so the format is read from the font rather than assumed, like `unitsPerEm`.
///
/// An empty range is not an error: `loca[gid] == loca[gid+1]` is how the format spells
/// "this glyph has no outline", which is exactly what a space is.
#[rustfmt::skip]
const GLYPH_LOCA: &str =
r#"FUNC __canvas_locFormat(b AS List OF Byte) AS Integer
  LET head AS Integer = __canvas_fontTable(b, "head")
  IF head < 0 THEN
    RETURN 0
  END IF
  RETURN __canvas_beU16(b, head + 50)
END FUNC

FUNC __canvas_locaAt(b AS List OF Byte, loca AS Integer, index AS Integer) AS Integer
  IF __canvas_locFormat(b) = 0 THEN
    RETURN __canvas_beU16(b, loca + index * 2) * 2
  END IF
  RETURN __canvas_beU32(b, loca + index * 4)
END FUNC

FUNC __canvas_glyphStart(b AS List OF Byte, gid AS Integer) AS Integer
  LET loca AS Integer = __canvas_fontTable(b, "loca")
  LET glyf AS Integer = __canvas_fontTable(b, "glyf")
  IF loca < 0 OR glyf < 0 THEN
    RETURN 0 - 1
  END IF
  LET from AS Integer = __canvas_locaAt(b, loca, gid)
  LET stop AS Integer = __canvas_locaAt(b, loca, gid + 1)
  IF stop <= from THEN
    RETURN 0 - 1
  END IF
  RETURN glyf + from
END FUNC"#;

/// Expand the flag array, which is run-length encoded.
///
/// Bit 3 (`REPEAT`) means "the next byte says how many *more* points share this flag".
/// Reading flags one per point without honouring it desynchronises the coordinate
/// arrays that follow, and the glyph comes out as noise rather than as nothing — the
/// failure mode that makes this worth its own helper.
///
/// The result carries the reader's final position as its last element, because the x
/// coordinates start wherever the flags stopped and MFBASIC returns one value.
#[rustfmt::skip]
const GLYPH_FLAGS: &str =
r#"FUNC __canvas_glyphFlags(b AS List OF Byte, at AS Integer, points AS Integer) AS List OF Integer
  MUT flags AS List OF Integer = []
  MUT cursor AS Integer = at
  WHILE len(flags) < points
    LET f AS Integer = __canvas_beU8(b, cursor)
    cursor = cursor + 1
    flags = collections::append(flags, f)
    IF (f / 8) MOD 2 = 1 THEN
      MUT repeat AS Integer = __canvas_beU8(b, cursor)
      cursor = cursor + 1
      WHILE repeat > 0 AND len(flags) < points
        flags = collections::append(flags, f)
        repeat = repeat - 1
      END WHILE
    END IF
  END WHILE
  RETURN collections::append(flags, cursor)
END FUNC"#;

/// Delta-decode one coordinate axis.
///
/// Two flag bits per axis and three cases between them: a short byte whose *sign* comes
/// from the second bit, a signed 16-bit delta, or — when the short bit is clear and the
/// same bit is set — no bytes at all, meaning "same as the previous point". That last
/// case is why a naive two-case reader drifts: it consumes bytes that were never there.
#[rustfmt::skip]
const GLYPH_COORDS: &str =
r#"FUNC __canvas_glyphCoords(b AS List OF Byte, at AS Integer, flags AS List OF Integer, points AS Integer, shortBit AS Integer, sameBit AS Integer) AS List OF Integer
  MUT out AS List OF Integer = []
  MUT cursor AS Integer = at
  MUT value AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < points
    LET f AS Integer = collections::getOr(flags, i, 0)
    LET isShort AS Boolean = (f / shortBit) MOD 2 = 1
    LET isSame AS Boolean = (f / sameBit) MOD 2 = 1
    IF isShort THEN
      LET d AS Integer = __canvas_beU8(b, cursor)
      cursor = cursor + 1
      IF isSame THEN
        value = value + d
      ELSE
        value = value - d
      END IF
    ELSE
      IF NOT isSame THEN
        value = value + __canvas_beS16(b, cursor)
        cursor = cursor + 2
      END IF
    END IF
    out = collections::append(out, value)
    i = i + 1
  END WHILE
  RETURN collections::append(out, cursor)
END FUNC"#;

/// Flatten one glyph's contours into edges, in surface pixels.
///
/// `scale` converts font units to pixels and `penX`/`penY` place the glyph; `penY` is
/// the **baseline**, and the Y axis flips here because a font's Y grows upward and the
/// surface's grows down. Doing the flip at this one point is what lets everything
/// downstream — bounds, coverage, stroke — stay in surface coordinates.
///
/// A quadratic segment becomes `__CANVAS_GLYPH_STEPS` straight ones. A fixed count
/// rather than one derived from the curve's size, because the geometry cache keys on
/// the item, not on the size, so a size-dependent count would make the same string
/// re-flatten whenever it moved. Six is where a 32-pixel glyph's error falls below the
/// quarter-pixel the coverage ramp can express.
///
/// Two on-curve points in a row are a straight edge; two off-curve points in a row
/// imply an on-curve point at their midpoint, which the format leaves out and every
/// reader has to put back.
#[rustfmt::skip]
const GLYPH_EDGES: &str =
r#"LET __CANVAS_GLYPH_STEPS AS Integer = 6

FUNC __canvas_glyphEdges(b AS List OF Byte, gid AS Integer, scale AS Float, penX AS Float, penY AS Float, edges AS List OF Float) AS List OF Float
  MUT out AS List OF Float = edges
  LET start AS Integer = __canvas_glyphStart(b, gid)
  IF start < 0 THEN
    RETURN out
  END IF
  LET contours AS Integer = __canvas_beS16(b, start)
  IF contours <= 0 THEN
    RETURN out
  END IF
  LET endsAt AS Integer = start + 10
  LET points AS Integer = __canvas_beU16(b, endsAt + (contours - 1) * 2) + 1
  LET instructions AS Integer = __canvas_beU16(b, endsAt + contours * 2)
  LET flagsAt AS Integer = endsAt + contours * 2 + 2 + instructions
  LET flags AS List OF Integer = __canvas_glyphFlags(b, flagsAt, points)
  LET xsAt AS Integer = collections::getOr(flags, points, 0)
  LET xs AS List OF Integer = __canvas_glyphCoords(b, xsAt, flags, points, 2, 16)
  LET ysAt AS Integer = collections::getOr(xs, points, 0)
  LET ys AS List OF Integer = __canvas_glyphCoords(b, ysAt, flags, points, 4, 32)
  MUT contour AS Integer = 0
  MUT from AS Integer = 0
  WHILE contour < contours
    LET last AS Integer = __canvas_beU16(b, endsAt + contour * 2)
    out = __canvas_contourEdges(flags, xs, ys, from, last, scale, penX, penY, out)
    from = last + 1
    contour = contour + 1
  END WHILE
  RETURN out
END FUNC"#;

/// One closed contour, walked point by point.
///
/// The walk starts at an on-curve point so the "previous on-curve" is always known. A
/// contour of nothing but off-curve points is legal — a circle drawn as four quadratics
/// has one — and its start is the midpoint of the last and first points, which is why
/// the search for a starting point falls back to synthesising one rather than giving up.
#[rustfmt::skip]
const CONTOUR_EDGES: &str =
r#"FUNC __canvas_onCurve(flags AS List OF Integer, i AS Integer) AS Boolean
  RETURN collections::getOr(flags, i, 0) MOD 2 = 1
END FUNC

FUNC __canvas_ptX(xs AS List OF Integer, i AS Integer, scale AS Float, penX AS Float) AS Float
  RETURN penX + toFloat(collections::getOr(xs, i, 0)) * scale
END FUNC

FUNC __canvas_ptY(ys AS List OF Integer, i AS Integer, scale AS Float, penY AS Float) AS Float
  RETURN penY - toFloat(collections::getOr(ys, i, 0)) * scale
END FUNC

FUNC __canvas_contourEdges(flags AS List OF Integer, xs AS List OF Integer, ys AS List OF Integer, from AS Integer, last AS Integer, scale AS Float, penX AS Float, penY AS Float, edges AS List OF Float) AS List OF Float
  MUT out AS List OF Float = edges
  LET count AS Integer = last - from + 1
  IF count < 2 THEN
    RETURN out
  END IF
  MUT startIndex AS Integer = 0 - 1
  MUT i AS Integer = 0
  WHILE i < count
    IF startIndex < 0 AND __canvas_onCurve(flags, from + i) THEN
      startIndex = i
    END IF
    i = i + 1
  END WHILE
  MUT curX AS Float = 0.0
  MUT curY AS Float = 0.0
  IF startIndex < 0 THEN
    ' Every point is off-curve: the contour starts at the midpoint of the last and
    ' first, which is the on-curve point the format leaves implied.
    startIndex = 0
    curX = (__canvas_ptX(xs, from, scale, penX) + __canvas_ptX(xs, last, scale, penX)) / 2.0
    curY = (__canvas_ptY(ys, from, scale, penY) + __canvas_ptY(ys, last, scale, penY)) / 2.0
  ELSE
    curX = __canvas_ptX(xs, from + startIndex, scale, penX)
    curY = __canvas_ptY(ys, from + startIndex, scale, penY)
  END IF
  LET originX AS Float = curX
  LET originY AS Float = curY
  MUT pendingX AS Float = 0.0
  MUT pendingY AS Float = 0.0
  MUT havePending AS Boolean = FALSE
  MUT walk AS Integer = 1
  WHILE walk <= count
    LET at AS Integer = from + (startIndex + walk) MOD count
    LET px AS Float = __canvas_ptX(xs, at, scale, penX)
    LET py AS Float = __canvas_ptY(ys, at, scale, penY)
    IF __canvas_onCurve(flags, at) THEN
      IF havePending THEN
        out = __canvas_quadEdges(out, curX, curY, pendingX, pendingY, px, py)
        havePending = FALSE
      ELSE
        out = __canvas_lineEdge(out, curX, curY, px, py)
      END IF
      curX = px
      curY = py
    ELSE
      IF havePending THEN
        ' Two control points in a row imply an on-curve point between them.
        LET midX AS Float = (pendingX + px) / 2.0
        LET midY AS Float = (pendingY + py) / 2.0
        out = __canvas_quadEdges(out, curX, curY, pendingX, pendingY, midX, midY)
        curX = midX
        curY = midY
      END IF
      pendingX = px
      pendingY = py
      havePending = TRUE
    END IF
    walk = walk + 1
  END WHILE
  IF havePending THEN
    out = __canvas_quadEdges(out, curX, curY, pendingX, pendingY, originX, originY)
  ELSE
    out = __canvas_lineEdge(out, curX, curY, originX, originY)
  END IF
  RETURN out
END FUNC

FUNC __canvas_quadEdges(edges AS List OF Float, x0 AS Float, y0 AS Float, cx AS Float, cy AS Float, x1 AS Float, y1 AS Float) AS List OF Float
  MUT out AS List OF Float = edges
  MUT prevX AS Float = x0
  MUT prevY AS Float = y0
  MUT i AS Integer = 1
  WHILE i <= __CANVAS_GLYPH_STEPS
    LET t AS Float = toFloat(i) / toFloat(__CANVAS_GLYPH_STEPS)
    LET u AS Float = 1.0 - t
    LET nx AS Float = u * u * x0 + 2.0 * u * t * cx + t * t * x1
    LET ny AS Float = u * u * y0 + 2.0 * u * t * cy + t * t * y1
    out = __canvas_lineEdge(out, prevX, prevY, nx, ny)
    prevX = nx
    prevY = ny
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_lineEdge(edges AS List OF Float, x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float) AS List OF Float
  LET dx AS Float = x1 - x0
  LET dy AS Float = y1 - y0
  LET lenSq AS Float = dx * dx + dy * dy
  IF lenSq <= 0.0 THEN
    RETURN edges
  END IF
  MUT out AS List OF Float = collections::append(edges, x0)
  out = collections::append(out, y0)
  out = collections::append(out, dx)
  out = collections::append(out, dy)
  RETURN collections::append(out, 1.0 / lenSq)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_glyphLoca", GLYPH_LOCA));
    pkg.add_helper(RegistryHelper::always("canvas_glyphFlags", GLYPH_FLAGS));
    pkg.add_helper(RegistryHelper::always("canvas_glyphCoords", GLYPH_COORDS));
    pkg.add_helper(RegistryHelper::always("canvas_glyphEdges", GLYPH_EDGES));
    pkg.add_helper(RegistryHelper::always("canvas_contourEdges", CONTOUR_EDGES));
}
