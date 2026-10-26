//! plan-176-S: `helper_render`'s MFBASIC text for the scene walks — `RENDER_SCENE` and `HASH_SCENE`. Registration lives in
//! `builtins/canvas/helper_render/mod.rs`.

/// Render the currently-installed scene into the surface buffer.
///
/// Reads the published scene rather than taking the item list as an argument: the
/// published copy is the one the renderer must draw, and re-reading it here keeps
/// "what was installed" and "what is drawn" the same object even when a later resize
/// or damage event re-renders without a `present`.
///
/// A layered scene and a flat one render through the same per-item path; the only
/// difference is that layers composite in order, which falls out of drawing them in
/// order onto one buffer.
///
/// Items are looked up by index alongside their published hashes, so an item and its
/// cache key stay paired. A hash the publish did not supply reads as `0`, which is a
/// key like any other: it costs a confirmation compare, never a wrong reuse.
#[rustfmt::skip]
pub(super) const RENDER_SCENE: &str =
r#"FUNC __canvas_renderScene(offsets AS List OF Integer, damage AS List OF Integer, width AS Integer, height AS Integer) AS Nothing
  LET full AS Boolean = __canvas_damageIsFull(damage, width, height)
  MUT buffer AS List OF Byte = []
  ' bug-1089: every item is drawn through the frame's scissor -- the whole surface, or
  ' the damage rectangle a partial redraw clears -- so a partial redraw writes no pixel
  ' it did not clear (`__CANVAS_SCISSOR_*`).
  __canvas_enterScissor(0, 0, width, height)
  IF full THEN
    buffer = canvas::newSurface(width, height)
  ELSE
    ' The previous frame's pixels, with only the damaged rectangle cleared back to the
    ' surface's own opaque black. Everything outside it is already correct -- that is
    ' the entire claim a partial redraw makes, and the reason the damage rectangle has
    ' to include where a moved item *was* as well as where it is.
    buffer = __CANVAS_KEPT
    LET x0 AS Integer = collections::getOr(damage, 0, 0)
    LET y0 AS Integer = collections::getOr(damage, 1, 0)
    LET x1 AS Integer = collections::getOr(damage, 2, 0)
    LET y1 AS Integer = collections::getOr(damage, 3, 0)
    __canvas_enterScissor(x0, y0, x1, y1)
    MUT y AS Integer = y0
    WHILE y < y1
      LET row AS Integer = y * width * 4
      MUT x AS Integer = x0
      WHILE x < x1
        LET at AS Integer = row + x * 4
        buffer = collections::set(buffer, at, toByte(0))
        buffer = collections::set(buffer, at + 1, toByte(0))
        buffer = collections::set(buffer, at + 2, toByte(0))
        buffer = collections::set(buffer, at + 3, toByte(255))
        x = x + 1
      END WHILE
      y = y + 1
    END WHILE
  END IF
  ' plan-116-G: indexed rather than `FOR EACH`, because each draw entry now carries an
  ' accumulated group translation alongside its geometry offset. The two travel in
  ' parallel globals written by `__canvas_sceneOffsets` -- one list per fact, indexed
  ' together. A scene with no groups leaves every entry at 0.0 and draws exactly what it
  ' drew before this letter.
  MUT di AS Integer = 0
  LET drawCount AS Integer = len(offsets)
  WHILE di < drawCount
    LET offset AS Integer = collections::getOr(offsets, di, 0)
    LET gdx AS Float = collections::getOr(__CANVAS_DRAW_DX, di, 0.0)
    LET gdy AS Float = collections::getOr(__CANVAS_DRAW_DY, di, 0.0)
    ' plan-172-G: the entry's group matrix, current for the bounds test and the draw.
    __canvas_enterGroupMatrix(collections::getOr(__CANVAS_DRAW_MAT, di, 0))
    IF __canvas_boundsMeetViewport(offset, width, height, gdx, gdy) AND (full OR __canvas_boundsMeetOffset(offset, damage, gdx, gdy)) THEN
      buffer = __canvas_drawGeometry(buffer, width, height, offset, gdx, gdy)
    END IF
    di = di + 1
  END WHILE
  __canvas_enterGroupMatrix(0)
  __canvas_enterScissor(0, 0, width, height)
  ' Kept only for damage mode, the one reader (`__canvas_damageFor` answers a full
  ' redraw otherwise): the assignment is a whole-frame copy -- 20 MB at 2880x1800,
  ' measured at 6 ms a frame -- paid on every frame for nothing (bug-686).
  IF __canvas_damageEnabled() THEN
    __CANVAS_KEPT = buffer
    __CANVAS_KEPT_W = width
    __CANVAS_KEPT_H = height
  END IF
  __canvas_presentSurface(buffer, width, height)
END FUNC"#;

/// The per-item content hashes for a scene, in item order.
///
/// A layered scene flattens into one hash list in draw order, which is exactly the
/// order `__canvas_renderScene` walks — so one index serves both shapes and the
/// renderer needs no shape-specific hash lookup.
#[rustfmt::skip]
pub(super) const HASH_SCENE: &str =
r#"' bug-686: `carried` is `canvas::carriedHashes(items)`, taken before the publish --
' the installed hash for every item whose bytes did not change, `-1` for the rest.
' `canvas::sceneHashes` keeps the carried ones and hashes the rest natively in one pass
' over the list, except a `Text` or a `Group`, which it leaves at `-1` for
' `__canvas_hashItem`'s MFBASIC arms below. So a scene of the common kinds costs the
' worker one native call and never extracts an item from the list. `items` and
' `carried` have the same length by construction; `sceneHashes` treats a short
' `carried` as all `-1`.
'
' plan-175-A: the list is `2n` words for `n` items -- every item's damage identity `D`
' at `i`, then every item's geometry identity `G` at `n + i`, one list under the one
' hashes revision, so the sequence lock covers both. Every reader that wants today's
' "the item's hash" reads index `i` and gets `D`.
FUNC __canvas_hashScene(items AS List OF DrawItem, carried AS List OF Integer) AS List OF Integer
  MUT out AS List OF Integer = canvas::sceneHashes(items, carried)
  LET count AS Integer = len(out) / 2
  MUT i AS Integer = 0
  WHILE i < count
    IF collections::getOr(out, i, 0) < 0 THEN
      ' Never read: every index below `count` is present. A group naming nothing
      ' would draw nothing, so it is also a harmless value if it ever were.
      LET none AS DrawItem = Group[id := "", position := vector::Float2[x := 0.0, y := 0.0], transform := __canvas_noTransform(), name := ""]
      LET item AS DrawItem = collections::getOr(items, i, none)
      LET g AS Integer = __canvas_geoHash(item)
      out = collections::set(out, i, __canvas_drawHashOf(item, g))
      out = collections::set(out, count + i, g)
    END IF
    i = i + 1
  END WHILE
  RETURN out
END FUNC

' A layered scene's hashes, in draw order, laid out as `__canvas_hashScene`'s: every
' item's `D`, then every item's `G`.
FUNC __canvas_hashLayers(layers AS List OF DrawLayer) AS List OF Integer
  MUT draws AS List OF Integer = []
  MUT geos AS List OF Integer = []
  FOR EACH layer IN layers
    FOR EACH item IN layer.items
      LET g AS Integer = __canvas_geoHash(item)
      draws = collections::append(draws, __canvas_drawHashOf(item, g))
      geos = collections::append(geos, g)
    NEXT
  NEXT
  RETURN collections::append(draws, geos)
END FUNC"#;
