//! plan-176-R: the `helper_particles` MFBASIC helper text — particle damage (`DAMAGE`).
//! Registration lives in `builtins/canvas/helper_particles/mod.rs`.

/// Damage, for a frame that holds a particle system (plan-150-B Phase 3).
///
/// The damage diff (`helper_damage.rs`) pairs this frame's draw entries with the last
/// frame's by index and answers a full redraw when the two frames have different
/// numbers of them. A system's entries are its LIVE particles, and that number changes
/// from frame to frame as particles are born and die — so a scene with a system in it
/// would be a full redraw on nearly every frame, whatever the particles covered.
///
/// So such a frame is diffed by UNIT, not by entry: every entry outside a system is a
/// unit by itself, exactly as before, and each system is ONE unit — its hash folded
/// from its particles' entry hashes, its bounds the union of theirs. The number of
/// units is the scene's structure and does not move with the live count, so the diff
/// lines up; a system that changed contributes one rectangle, where its particles were
/// and where they are.
///
/// All of it is off unless damage is on, which only a `--debug` build can be
/// (`MFB_CANVAS_DAMAGE`): the spans are not even recorded otherwise.
#[rustfmt::skip]
pub(super) const DAMAGE: &str =
r#"' The systems the frame's walk expanded, in walk order.
'
' `__CANVAS_PARTICLE_RAW` is (first entry, count) in the walk's own list, for the scene
' index being walked. `__canvas_particleOwned` moves them to `__CANVAS_PARTICLE_OWNED`
' as (scene index, first entry within that index's entries, count), and
' `__canvas_particleLayout` turns those into `__CANVAS_PARTICLE_SPANS`: (first entry,
' count) in the frame's FINAL draw entries, which is what the diff reads.
MUT __CANVAS_PARTICLE_RAW AS List OF Integer = []
MUT __CANVAS_PARTICLE_OWNED AS List OF Integer = []
MUT __CANVAS_PARTICLE_SPANS AS List OF Integer = []

FUNC __canvas_particleBegin() AS Nothing
  IF len(__CANVAS_PARTICLE_RAW) > 0 THEN
    __CANVAS_PARTICLE_RAW = []
  END IF
  IF len(__CANVAS_PARTICLE_OWNED) > 0 THEN
    __CANVAS_PARTICLE_OWNED = []
  END IF
  IF len(__CANVAS_PARTICLE_SPANS) > 0 THEN
    __CANVAS_PARTICLE_SPANS = []
  END IF
END FUNC

' One system's entries in the walk's list: noted only in damage mode.
FUNC __canvas_particleNote(first AS Integer, count AS Integer) AS Nothing
  IF __canvas_damageEnabled() THEN
    __CANVAS_PARTICLE_RAW = collections::append(__CANVAS_PARTICLE_RAW, first)
    __CANVAS_PARTICLE_RAW = collections::append(__CANVAS_PARTICLE_RAW, count)
  END IF
END FUNC

' The systems noted while scene index `index` was walked, whose entries start at
' `before` in the walk's list.
FUNC __canvas_particleOwned(index AS Integer, before AS Integer) AS Nothing
  LET raw AS Integer = len(__CANVAS_PARTICLE_RAW)
  IF raw = 0 THEN
    RETURN
  END IF
  MUT k AS Integer = 0
  WHILE k + 1 < raw
    __CANVAS_PARTICLE_OWNED = collections::append(__CANVAS_PARTICLE_OWNED, index)
    __CANVAS_PARTICLE_OWNED = collections::append(__CANVAS_PARTICLE_OWNED, collections::getOr(__CANVAS_PARTICLE_RAW, k, 0) - before)
    __CANVAS_PARTICLE_OWNED = collections::append(__CANVAS_PARTICLE_OWNED, collections::getOr(__CANVAS_PARTICLE_RAW, k + 1, 0))
    k = k + 2
  END WHILE
  __CANVAS_PARTICLE_RAW = []
END FUNC

' Where each noted system's entries are in the frame's final draw entries. `spans` is
' what `canvas::sceneLayout` was given: (scene index, first entry in the walk's list,
' entry count) for every index the walk expanded, in order. An index's entries start at
' the index itself plus one less than the count of every span before it.
FUNC __canvas_particleLayout(spans AS List OF Integer) AS Nothing
  LET owned AS Integer = len(__CANVAS_PARTICLE_OWNED)
  IF owned = 0 THEN
    RETURN
  END IF
  MUT extra AS Integer = 0
  MUT s AS Integer = 0
  MUT k AS Integer = 0
  WHILE k + 2 < owned
    LET index AS Integer = collections::getOr(__CANVAS_PARTICLE_OWNED, k, 0)
    WHILE s + 2 < len(spans) AND collections::getOr(spans, s, 0) < index
      extra = extra + collections::getOr(spans, s + 2, 0) - 1
      s = s + 3
    END WHILE
    __CANVAS_PARTICLE_SPANS = collections::append(__CANVAS_PARTICLE_SPANS, index + extra + collections::getOr(__CANVAS_PARTICLE_OWNED, k + 1, 0))
    __CANVAS_PARTICLE_SPANS = collections::append(__CANVAS_PARTICLE_SPANS, collections::getOr(__CANVAS_PARTICLE_OWNED, k + 2, 0))
    k = k + 3
  END WHILE
END FUNC

' Whether this frame is diffed by unit: it expanded a system, in damage mode.
FUNC __canvas_particleFramed() AS Boolean
  RETURN len(__CANVAS_PARTICLE_SPANS) > 0
END FUNC

' The frame's units, as (first entry, count) pairs covering its `count` entries in
' order: each system's span, and every entry between them alone.
FUNC __canvas_particleUnits(count AS Integer) AS List OF Integer
  LET spans AS List OF Integer = __CANVAS_PARTICLE_SPANS
  LET total AS Integer = len(spans)
  MUT units AS List OF Integer = []
  MUT i AS Integer = 0
  MUT s AS Integer = 0
  WHILE i < count OR s + 1 < total
    IF s + 1 < total AND collections::getOr(spans, s, 0) <= i THEN
      LET n AS Integer = collections::getOr(spans, s + 1, 0)
      units = collections::append(units, i)
      units = collections::append(units, n)
      i = i + n
      s = s + 2
    ELSEIF i < count THEN
      units = collections::append(units, i)
      units = collections::append(units, 1)
      i = i + 1
    ELSE
      s = total
    END IF
  END WHILE
  RETURN units
END FUNC

' A unit's hash: an entry's own when it is one entry -- so a frame of single entries
' remembers exactly the hashes it always did -- and otherwise its entries' hashes folded
' in order after their count, so a system with no live particle is still a value.
FUNC __canvas_particleUnitHash(hashes AS List OF Integer, first AS Integer, n AS Integer) AS Integer
  IF n = 1 THEN
    RETURN collections::getOr(hashes, first, 0)
  END IF
  MUT h AS Integer = __canvas_hashStep(__canvas_hashStart(15), n)
  MUT k AS Integer = 0
  WHILE k < n
    h = __canvas_hashStep(h, collections::getOr(hashes, first + k, 0))
    k = k + 1
  END WHILE
  RETURN h
END FUNC

' A unit's bounds on the surface -- the union of its entries', each through its own
' group offset and matrix, as `__canvas_rememberScene` takes one entry's -- in
' `__CANVAS_PARTICLE_BOX`; all zero for a unit that draws nothing.
MUT __CANVAS_PARTICLE_BOX AS List OF Float = [0.0, 0.0, 0.0, 0.0]

FUNC __canvas_particleUnitBox(offsets AS List OF Integer, first AS Integer, n AS Integer) AS Nothing
  MUT any AS Boolean = FALSE
  MUT x0 AS Float = 0.0
  MUT y0 AS Float = 0.0
  MUT x1 AS Float = 0.0
  MUT y1 AS Float = 0.0
  MUT k AS Integer = first
  WHILE k < first + n
    LET offset AS Integer = collections::getOr(offsets, k, 0)
    LET gdx AS Float = collections::getOr(__CANVAS_DRAW_DX, k, 0.0)
    LET gdy AS Float = collections::getOr(__CANVAS_DRAW_DY, k, 0.0)
    __canvas_enterGroupMatrix(collections::getOr(__CANVAS_DRAW_MAT, k, 0))
    LET bx0 AS Float = __canvas_drawBound(offset, 0, gdx, gdy)
    LET by0 AS Float = __canvas_drawBound(offset, 1, gdx, gdy)
    LET bx1 AS Float = __canvas_drawBound(offset, 2, gdx, gdy)
    LET by1 AS Float = __canvas_drawBound(offset, 3, gdx, gdy)
    IF bx1 > bx0 AND by1 > by0 THEN
      IF NOT any THEN
        x0 = bx0
        y0 = by0
        x1 = bx1
        y1 = by1
        any = TRUE
      ELSE
        x0 = __canvas_minF(x0, bx0)
        y0 = __canvas_minF(y0, by0)
        x1 = __canvas_maxF(x1, bx1)
        y1 = __canvas_maxF(y1, by1)
      END IF
    END IF
    k = k + 1
  END WHILE
  __canvas_enterGroupMatrix(0)
  __CANVAS_PARTICLE_BOX = [x0, y0, x1, y1]
END FUNC

' `__canvas_rememberScene` for a frame diffed by unit: one hash and one rectangle a
' unit, in the lists the entry diff keeps one an entry in.
FUNC __canvas_particleRemember(hashes AS List OF Integer, offsets AS List OF Integer) AS Nothing
  LET units AS List OF Integer = __canvas_particleUnits(len(offsets))
  MUT kept AS List OF Integer = []
  MUT bounds AS List OF Float = []
  MUT u AS Integer = 0
  WHILE u + 1 < len(units)
    LET first AS Integer = collections::getOr(units, u, 0)
    LET n AS Integer = collections::getOr(units, u + 1, 0)
    kept = collections::append(kept, __canvas_particleUnitHash(hashes, first, n))
    __canvas_particleUnitBox(offsets, first, n)
    FOR EACH edge IN __CANVAS_PARTICLE_BOX
      bounds = collections::append(bounds, edge)
    NEXT
    u = u + 2
  END WHILE
  __CANVAS_LAST_HASHES = kept
  __CANVAS_LAST_BOUNDS = bounds
END FUNC

' `__canvas_damageFor` for a frame diffed by unit, after its own checks that there is a
' kept surface of this size: a full frame when the last frame had a different number of
' units -- the scene's structure changed -- and otherwise the union, over every unit
' whose hash changed, of where it was and where it is.
FUNC __canvas_particleDamage(hashes AS List OF Integer, offsets AS List OF Integer, width AS Integer, height AS Integer) AS List OF Integer
  LET units AS List OF Integer = __canvas_particleUnits(len(offsets))
  LET count AS Integer = len(units) / 2
  IF len(__CANVAS_LAST_HASHES) <> count OR len(__CANVAS_LAST_BOUNDS) <> count * 4 THEN
    RETURN __canvas_damageFull(width, height)
  END IF
  MUT minX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT maxY AS Float = 0.0
  MUT any AS Boolean = FALSE
  MUT u AS Integer = 0
  WHILE u < count
    LET first AS Integer = collections::getOr(units, u * 2, 0)
    LET n AS Integer = collections::getOr(units, u * 2 + 1, 0)
    IF __canvas_particleUnitHash(hashes, first, n) <> collections::getOr(__CANVAS_LAST_HASHES, u, 0) THEN
      __canvas_particleUnitBox(offsets, first, n)
      FOR EACH box IN [__CANVAS_PARTICLE_BOX, [collections::getOr(__CANVAS_LAST_BOUNDS, u * 4, 0.0), collections::getOr(__CANVAS_LAST_BOUNDS, u * 4 + 1, 0.0), collections::getOr(__CANVAS_LAST_BOUNDS, u * 4 + 2, 0.0), collections::getOr(__CANVAS_LAST_BOUNDS, u * 4 + 3, 0.0)]]
        LET bx0 AS Float = collections::getOr(box, 0, 0.0)
        LET by0 AS Float = collections::getOr(box, 1, 0.0)
        LET bx1 AS Float = collections::getOr(box, 2, 0.0)
        LET by1 AS Float = collections::getOr(box, 3, 0.0)
        IF bx1 > bx0 AND by1 > by0 THEN
          IF NOT any THEN
            minX = bx0
            minY = by0
            maxX = bx1
            maxY = by1
            any = TRUE
          ELSE
            minX = __canvas_minF(minX, bx0)
            minY = __canvas_minF(minY, by0)
            maxX = __canvas_maxF(maxX, bx1)
            maxY = __canvas_maxF(maxY, by1)
          END IF
        END IF
      NEXT
    END IF
    u = u + 1
  END WHILE
  IF NOT any THEN
    RETURN []
  END IF
  ' Rounded outward and clamped, as the entry diff rounds its own.
  MUT x0 AS Integer = toInt(__canvas_floorF(minX)) - 1
  MUT y0 AS Integer = toInt(__canvas_floorF(minY)) - 1
  MUT x1 AS Integer = toInt(__canvas_floorF(maxX)) + 2
  MUT y1 AS Integer = toInt(__canvas_floorF(maxY)) + 2
  IF x0 < 0 THEN
    x0 = 0
  END IF
  IF y0 < 0 THEN
    y0 = 0
  END IF
  IF x1 > width THEN
    x1 = width
  END IF
  IF y1 > height THEN
    y1 = height
  END IF
  IF x1 <= x0 OR y1 <= y0 THEN
    RETURN []
  END IF
  RETURN [x0, y0, x1, y1]
END FUNC"#;
