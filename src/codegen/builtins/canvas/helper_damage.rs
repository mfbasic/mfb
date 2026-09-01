//! Damage: what changed between the last rendered frame and this one (plan-98-G Phase 3).
//!
//! **The mechanism the plan named does not exist in this design.**
//! `VK_KHR_incremental_present` and Metal dirty rects are properties of *presenting a
//! swapchain drawable*, and this renderer presents no drawable: it renders offscreen,
//! reads the pixels back, and hands them to the platform's own surface
//! (plan-98-F Correction 1). So damage is consumed one layer down, where this design
//! actually spends its time — see Correction 24.
//!
//! Two consumers, and the first is worth more than the second:
//!
//! * **Nothing changed → no frame at all.** No rasterisation, no GPU submit, no
//!   readback, no blit. A program that re-presents an unchanged scene on every vsync
//!   does no work, which is the strongest possible reading of "present only the damage
//!   union" — the union is empty, so nothing is presented.
//! * **Something changed → redraw only its rectangle.** The previous frame's pixels are
//!   kept, the damaged rectangle is cleared, and only the items whose bounds meet it are
//!   drawn again. A one-word label changing in the corner of a 900x640 window costs its
//!   own area rather than the window's.
//!
//! **Why the union and not a list of rectangles.** A scene diff can produce many
//! disjoint damaged areas, and tracking them separately would let a two-corner change
//! skip the middle. It would also make every consumer — the clear, the intersection
//! test, the stats line — take a list where it now takes four numbers, for a saving that
//! only appears in scenes that change in several places at once, which is exactly the
//! case where the union is large anyway.
//!
//! Registered via `add_helper`; body byte-significant (2-space indent → `.ncode`
//! columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// What the renderer remembers between frames.
///
/// `__CANVAS_KEPT` is the previous frame's pixels — the thing that makes a partial
/// redraw possible at all. It is invalidated by a size change rather than resized: a
/// surface of the wrong size has no correct interpretation, and the next frame is a full
/// one anyway.
#[rustfmt::skip]
const DAMAGE_STATE: &str =
r#"MUT __CANVAS_LAST_HASHES AS List OF Integer = []
MUT __CANVAS_LAST_BOUNDS AS List OF Float = []
MUT __CANVAS_KEPT AS List OF Byte = []
MUT __CANVAS_KEPT_W AS Integer = 0
MUT __CANVAS_KEPT_H AS Integer = 0
MUT __CANVAS_FRAMES AS Integer = 0
MUT __CANVAS_SKIPPED AS Integer = 0
MUT __CANVAS_PARTIAL AS Integer = 0
MUT __CANVAS_DAMAGE AS List OF Integer = []

' Damage is off unless `MFB_CANVAS_DAMAGE` is set. It changes no visible output -- that
' is the property its tests assert -- but it does change *when* the renderer runs at all,
' and a frame counter that silently stops advancing is the kind of thing a stale test
' reads as a pass. Resolved once and cached; 0 is unresolved, 1 off, 2 on.
MUT __CANVAS_DAMAGE_MODE AS Integer = 0

FUNC __canvas_damageEnabled() AS Boolean
  IF __CANVAS_DAMAGE_MODE = 0 THEN
    __CANVAS_DAMAGE_MODE = 1
    IF len(os::getEnvOr("MFB_CANVAS_DAMAGE", "")) > 0 THEN
      __CANVAS_DAMAGE_MODE = 2
    END IF
  END IF
  RETURN __CANVAS_DAMAGE_MODE = 2
END FUNC"#;

/// The diff.
///
/// Returns `[]` when nothing changed, or `[x0, y0, x1, y1]` in whole pixels, clamped to
/// the surface. A full frame is the whole-surface rectangle rather than a separate
/// answer — every consumer then has one shape to handle, and "full" is a rectangle that
/// happens to be everything.
///
/// **Every case it cannot compare is a full frame**, and the list is short because each
/// entry is a place where a partial redraw would be *wrong* rather than merely
/// suboptimal: no kept surface, a different surface size, a different item count, and
/// damage switched off. That asymmetry is deliberate — a wrong full frame does not
/// exist, and a wrong partial frame is a stale rectangle nobody will notice until it is
/// on screen.
#[rustfmt::skip]
const DAMAGE_FOR: &str =
r#"FUNC __canvas_damageFull(width AS Integer, height AS Integer) AS List OF Integer
  RETURN [0, 0, width, height]
END FUNC

FUNC __canvas_damageFor(hashes AS List OF Integer, offsets AS List OF Integer, width AS Integer, height AS Integer) AS List OF Integer
  IF NOT __canvas_damageEnabled() THEN
    RETURN __canvas_damageFull(width, height)
  END IF
  IF len(__CANVAS_KEPT) <> width * height * 4 THEN
    RETURN __canvas_damageFull(width, height)
  END IF
  IF __CANVAS_KEPT_W <> width OR __CANVAS_KEPT_H <> height THEN
    RETURN __canvas_damageFull(width, height)
  END IF
  LET count AS Integer = len(offsets)
  IF len(__CANVAS_LAST_HASHES) <> count THEN
    RETURN __canvas_damageFull(width, height)
  END IF
  IF len(__CANVAS_LAST_BOUNDS) <> count * 4 THEN
    RETURN __canvas_damageFull(width, height)
  END IF

  MUT minX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT maxY AS Float = 0.0
  MUT any AS Boolean = FALSE
  MUT i AS Integer = 0
  WHILE i < count
    IF collections::getOr(hashes, i, 0) <> collections::getOr(__CANVAS_LAST_HASHES, i, 0) THEN
      ' Both rectangles. An item that moved has to erase where it was as well as paint
      ' where it is, and the two are the same rectangle only when it did not move.
      LET offset AS Integer = collections::getOr(offsets, i, 0)
      FOR EACH box IN [[__canvas_geoAt(offset, 16), __canvas_geoAt(offset, 17), __canvas_geoAt(offset, 18), __canvas_geoAt(offset, 19)], [collections::getOr(__CANVAS_LAST_BOUNDS, i * 4, 0.0), collections::getOr(__CANVAS_LAST_BOUNDS, i * 4 + 1, 0.0), collections::getOr(__CANVAS_LAST_BOUNDS, i * 4 + 2, 0.0), collections::getOr(__CANVAS_LAST_BOUNDS, i * 4 + 3, 0.0)]]
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
    i = i + 1
  END WHILE

  IF NOT any THEN
    RETURN []
  END IF
  ' One pixel of margin on each side, then clamped. The bounds are the rasteriser's own
  ' -- they already include the antialiasing rim -- but they are floats and the clear is
  ' in whole pixels, so rounding outward is what keeps a partial redraw from leaving a
  ' one-pixel ghost along an edge.
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
END FUNC

FUNC __canvas_damageIsFull(damage AS List OF Integer, width AS Integer, height AS Integer) AS Boolean
  IF len(damage) < 4 THEN
    RETURN FALSE
  END IF
  RETURN collections::getOr(damage, 0, 0) <= 0 AND collections::getOr(damage, 1, 0) <= 0 AND collections::getOr(damage, 2, 0) >= width AND collections::getOr(damage, 3, 0) >= height
END FUNC

' Whether an item's geometry meets the damaged rectangle. An item that does not is not
' redrawn -- its pixels are already in the kept surface, and redrawing it would be the
' whole cost the damage rectangle exists to avoid.
FUNC __canvas_boundsMeet(offset AS Integer, damage AS List OF Integer) AS Boolean
  LET x0 AS Float = __canvas_geoAt(offset, 16)
  LET y0 AS Float = __canvas_geoAt(offset, 17)
  LET x1 AS Float = __canvas_geoAt(offset, 18)
  LET y1 AS Float = __canvas_geoAt(offset, 19)
  IF x1 <= x0 OR y1 <= y0 THEN
    RETURN FALSE
  END IF
  IF x1 < toFloat(collections::getOr(damage, 0, 0)) THEN
    RETURN FALSE
  END IF
  IF y1 < toFloat(collections::getOr(damage, 1, 0)) THEN
    RETURN FALSE
  END IF
  IF x0 > toFloat(collections::getOr(damage, 2, 0)) THEN
    RETURN FALSE
  END IF
  IF y0 > toFloat(collections::getOr(damage, 3, 0)) THEN
    RETURN FALSE
  END IF
  RETURN TRUE
END FUNC

' Remember what was drawn, so the next frame has something to diff against.
SUB __canvas_rememberScene(hashes AS List OF Integer, offsets AS List OF Integer)
  MUT bounds AS List OF Float = []
  FOR EACH offset IN offsets
    bounds = collections::append(bounds, __canvas_geoAt(offset, 16))
    bounds = collections::append(bounds, __canvas_geoAt(offset, 17))
    bounds = collections::append(bounds, __canvas_geoAt(offset, 18))
    bounds = collections::append(bounds, __canvas_geoAt(offset, 19))
  NEXT
  __CANVAS_LAST_BOUNDS = bounds
  MUT kept AS List OF Integer = []
  FOR EACH h IN hashes
    kept = collections::append(kept, h)
  NEXT
  __CANVAS_LAST_HASHES = kept
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_damageState", DAMAGE_STATE));
    pkg.add_helper(RegistryHelper::always("canvas_damageFor", DAMAGE_FOR));
}
