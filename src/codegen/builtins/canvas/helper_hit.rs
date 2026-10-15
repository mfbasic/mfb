//! Hit-testing (plan-172-I): which drawn item is topmost under a surface point, and
//! `canvas::pollMouse`, its first user.
//!
//! The test is built on the rasteriser's own distance functions — `__canvas_geoDistance`,
//! the clip distances and the inverse maps in `helper_items.rs` — read from the same
//! geometry records, so "is this point on the shape" is answered by the code that decides
//! which pixels the shape covers. It adds no distance function of its own.
//!
//! Two chunks, both gated, so a canvas program that uses neither compiles none of it:
//!
//! * [`REGION`] — "is this point on this draw entry". Compiled into a program that calls
//!   `pollMouse` **or constructs a `canvas::AutoPaint`** (plan-172-K), because it is the
//!   one test both read: the worker for `overID`, the graphics thread for which
//!   `AutoPaint` the pointer is on. One function, so the two cannot disagree.
//! * [`BODY`] — the worker's draw list, the id paths and `canvas::pollMouse` itself.
//!   Compiled only into a program that calls `pollMouse`, and so the only thing that
//!   emits the native raw poll or reserves mouse state.
//!
//! **Threads.** Everything here runs on the thread that calls it, against that thread's
//! own geometry cache and draw-entry lists (`.ai/canvas-threading.md` section 2: module
//! globals are per-thread). `canvas::pollMouse` runs on the worker, so the draw list
//! `__canvas_hitBuild` makes is the worker's, built from the scene it last presented —
//! nothing is read from the graphics thread and nothing waits on a frame. The graphics
//! thread hands [`REGION`] its own frame's entries (`helper_autopaint.rs`).
//!
//! **A `Text` is its layout box on both threads, and the box is found differently.**
//! The worker draws nothing, so its `Text` record is `__canvas_textBoxHeader`'s: the
//! box and no glyph. The graphics thread's is the renderer's — a glyph run, or the
//! polygon a stroked run is drawn as — so in a program with an `AutoPaint` the builder
//! writes the box into that record too (`__canvas_autoTextBox`), in the same four
//! slots.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

/// The region test: is a surface point on a draw entry.
#[rustfmt::skip]
const REGION: &str =
r#"IMPORT canvas
IMPORT collections
IMPORT math
IMPORT vector

' A text record's LAYOUT BOX -- x0, y0, x1, y1 in the item's own space: the pen's travel
' by the font's ascent and descent. It takes the gradient's four point slots, which a
' `Text` record never uses (`__canvas_paintHeader` gives that kind no gradient, and
' every reader of those slots asks the stop count first).
LET __CANVAS_GEO_TEXT_BOX AS Integer = 43
' plan-172-K: a STROKED `Text` is drawn as a polygon of its glyph outlines, and a
' polygon may use those slots for a gradient. So the polygon says it carries a box:
' this value in the gradient-kind slot, which a gradient writes 0 or 1 into -- and only
' on a record with fewer than two stops, where nothing reads that slot or the points.
' A stroked run never has more: `__canvas_textHeader` clears the count, because a
' gradient on a stroked `Text` is ignored.
LET __CANVAS_GEO_TEXT_BOX_MARK AS Integer = 2

' Is the record at `offset` hit as a text layout box?
FUNC __canvas_hitIsTextBox(offset AS Integer, kind AS Integer) AS Boolean
  IF kind = __CANVAS_GEO_TEXT THEN
    RETURN TRUE
  END IF
  IF kind <> __CANVAS_GEO_POLYGON THEN
    RETURN FALSE
  END IF
  RETURN toInt(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_KIND)) = __CANVAS_GEO_TEXT_BOX_MARK AND toInt(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_COUNT)) < 2
END FUNC

' The shape-space distance of a record at its own point (`qx`, `qy`): the value the
' rasteriser's fill reads, with the header's per-item constants set up as
' `__canvas_drawGeometry` sets them up. A dashed stroke is measured as the solid one it
' is cut from -- a gap between two dashes is still on the line.
FUNC __canvas_hitDistance(offset AS Integer, kind AS Integer, qx AS Float, qy AS Float) AS Float
  MUT sx AS Float = 0.0
  MUT sy AS Float = 0.0
  MUT ex AS Float = 0.0
  MUT ey AS Float = 0.0
  MUT reflex AS Boolean = FALSE
  IF kind = __CANVAS_GEO_ARC THEN
    LET startAngle AS Float = __canvas_geoAt(offset, 20)
    LET endAngle AS Float = __canvas_geoAt(offset, 21)
    reflex = (endAngle - startAngle) > 3.141592653589793
    sx = __canvas_cos(startAngle)
    sy = __canvas_sin(startAngle)
    ex = __canvas_cos(endAngle)
    ey = __canvas_sin(endAngle)
  END IF
  ' A picture is a rectangle to the distance function (bug-484).
  MUT distKind AS Integer = kind
  IF kind = __CANVAS_GEO_PICTURE THEN
    distKind = __CANVAS_KIND_RECT
  END IF
  RETURN __canvas_geoDistance(distKind, offset + __CANVAS_GEO_HEADER, toInt(__canvas_geoAt(offset, 20)), qx, qy, __canvas_geoAt(offset, 2), __canvas_geoAt(offset, 3), __canvas_geoAt(offset, 4), __canvas_geoAt(offset, 5), __canvas_geoAt(offset, 6), sx, sy, ex, ey, reflex, toInt(__canvas_geoAt(offset, __CANVAS_GEO_CAP)), __canvas_geoAt(offset, __CANVAS_GEO_CAPSTARTX), __canvas_geoAt(offset, __CANVAS_GEO_CAPSTARTY), __canvas_geoAt(offset, __CANVAS_GEO_CAPENDX), __canvas_geoAt(offset, __CANVAS_GEO_CAPENDY), __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_COS), __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_SIN), FALSE)
END FUNC

' The test for the entry whose group matrix is current (`__canvas_enterGroupMatrix`).
FUNC __canvas_hitCurrent(offset AS Integer, kind AS Integer, gdx AS Float, gdy AS Float, x AS Float, y AS Float) AS Boolean
  ' 1. The entry's surface box, widened by the slack an open kind is given. Not for a
  ' text box: a record's bounds are what it DRAWS, and on the graphics thread a stroked
  ' run's are its outlines' -- tighter than the layout box it is hit on, whose top is
  ' the font's ascent whatever the glyphs reach (plan-172-K). The box test below is the
  ' whole answer for those and costs four comparisons.
  LET isBox AS Boolean = __canvas_hitIsTextBox(offset, kind)
  IF NOT isBox THEN
    IF x < __canvas_drawBound(offset, 0, gdx, gdy) - 3.0 OR x > __canvas_drawBound(offset, 2, gdx, gdy) + 3.0 THEN
      RETURN FALSE
    END IF
    IF y < __canvas_drawBound(offset, 1, gdx, gdy) - 3.0 OR y > __canvas_drawBound(offset, 3, gdx, gdy) + 3.0 THEN
      RETURN FALSE
    END IF
  END IF
  ' 2. The item's own point: the group's translation off, then the groups' inverse
  ' linear part and the item's own inverse -- the rasteriser's `px`/`py` and `qx`/`qy`.
  LET px AS Float = x - gdx
  LET py AS Float = y - gdy
  LET transformed AS Boolean = __canvas_hasTransform(offset)
  MUT qx AS Float = px
  MUT qy AS Float = py
  IF transformed THEN
    qx = __canvas_invX(offset, px, py)
    qy = __canvas_invY(offset, px, py)
  END IF
  ' 3. The item's own clip, which is in its coordinates: outside it there is no item.
  LET clipMode AS Float = __canvas_geoAt(offset, __CANVAS_GEO_CLIP_MODE)
  IF clipMode = 2.0 THEN
    IF __canvas_clipPathDistance(offset, qx, qy) > 0.0 THEN
      RETURN FALSE
    END IF
  ELSEIF clipMode <> 0.0 THEN
    IF __canvas_clipRectDistance(offset, qx, qy) > 0.0 THEN
      RETURN FALSE
    END IF
  END IF
  ' 4. The shape. A `Text` is its layout box, whatever its glyphs cover.
  IF isBox THEN
    RETURN qx >= __canvas_geoAt(offset, __CANVAS_GEO_TEXT_BOX) AND qx <= __canvas_geoAt(offset, __CANVAS_GEO_TEXT_BOX + 2) AND qy >= __canvas_geoAt(offset, __CANVAS_GEO_TEXT_BOX + 1) AND qy <= __canvas_geoAt(offset, __CANVAS_GEO_TEXT_BOX + 3)
  END IF
  LET d AS Float = __canvas_hitDistance(offset, kind, qx, qy)
  IF kind = __CANVAS_KIND_SEGMENT OR kind = __CANVAS_GEO_ARC OR kind = __CANVAS_GEO_POLYLINE OR kind = __CANVAS_GEO_POLYCURVE THEN
    ' An open kind is its stroke: `d` is already the distance to the band's edge, the
    ' half-width (slot 6) taken off. A thin stroke is widened to 3 surface pixels either
    ' side of its centre line. `scale` is shape units per surface pixel, by the central
    ' differences the rasteriser divides a transformed distance by.
    IF d <= 0.0 THEN
      RETURN TRUE
    END IF
    MUT scale AS Float = 1.0
    IF transformed THEN
      LET gx AS Float = __canvas_hitDistance(offset, kind, __canvas_invX(offset, px + 0.5, py), __canvas_invY(offset, px + 0.5, py)) - __canvas_hitDistance(offset, kind, __canvas_invX(offset, px - 0.5, py), __canvas_invY(offset, px - 0.5, py))
      LET gy AS Float = __canvas_hitDistance(offset, kind, __canvas_invX(offset, px, py + 0.5), __canvas_invY(offset, px, py + 0.5)) - __canvas_hitDistance(offset, kind, __canvas_invX(offset, px, py - 0.5), __canvas_invY(offset, px, py - 0.5))
      LET g AS Float = math::sqrt(gx * gx + gy * gy)
      IF g > 0.000001 THEN
        scale = g
      END IF
    END IF
    RETURN d <= 3.0 * scale - __canvas_geoAt(offset, 6)
  END IF
  ' A closed kind is its interior and its stroke, drawn or not: the stroke's band is
  ' `|d| - half`, so both together are `d <= half`. A Miter or Bevel join reaches past
  ' that band at a corner, and is measured as the rasteriser measures it.
  LET half AS Float = __canvas_geoAt(offset, 7)
  IF d <= __canvas_maxF(half, 0.0) THEN
    RETURN TRUE
  END IF
  IF half > 0.0 AND toInt(__canvas_geoAt(offset, __CANVAS_GEO_JOIN_COUNT)) > 0 THEN
    RETURN __canvas_joinedStroke(kind, offset, offset + __CANVAS_GEO_HEADER, toInt(__canvas_geoAt(offset, 20)), qx, qy, __canvas_geoAt(offset, 2), __canvas_geoAt(offset, 3), __canvas_geoAt(offset, 4), __canvas_geoAt(offset, 5), half) <= 0.0
  END IF
  RETURN FALSE
END FUNC

' Is the surface point (`x`, `y`) on the draw entry at geometry `offset`, drawn under
' group matrix `mat` (0 for none) at group translation (`gdx`, `gdy`)?
FUNC __canvas_hitEntry(offset AS Integer, mat AS Integer, gdx AS Float, gdy AS Float, x AS Float, y AS Float) AS Boolean
  LET kind AS Integer = toInt(__canvas_geoAt(offset, 0))
  IF kind = __CANVAS_GEO_NONE THEN
    RETURN FALSE
  END IF
  __canvas_enterGroupMatrix(mat)
  LET hit AS Boolean = __canvas_hitCurrent(offset, kind, gdx, gdy, x, y)
  __canvas_enterGroupMatrix(0)
  RETURN hit
END FUNC"#;

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT canvas
IMPORT collections
IMPORT math
IMPORT vector

' The draw list the calling thread hit-tests: one geometry offset per entry, in draw
' order, parallel to `__CANVAS_DRAW_DX` / `_DY` / `_MAT` (the entry's group translation
' and matrix number, as `__canvas_composeGroup` leaves them) and to `__CANVAS_HIT_IDS`
' (its id path).
MUT __CANVAS_HIT_OFFSETS AS List OF Integer = []
MUT __CANVAS_HIT_IDS AS List OF String = []
' The scene revision (`__CANVAS_SCENE_REV`) the list was built for; 0 is "never".
MUT __CANVAS_HIT_REV AS Integer = 0
' Whether any entry of the list has a non-empty id path. FALSE answers every point with
' "" without a walk.
MUT __CANVAS_HIT_NAMED AS Boolean = FALSE

' Two parts of an id path, joined by the language's namespace separator. An empty part
' is left out, so a scene with no named group records exactly each item's own `id`.
FUNC __canvas_joinId(path AS String, id AS String) AS String
  IF id = "" THEN
    RETURN path
  END IF
  IF path = "" THEN
    RETURN id
  END IF
  RETURN path & "::" & id
END FUNC

FUNC __canvas_itemId(item AS DrawItem) AS String
  MATCH item
    CASE Rectangle(r)
      RETURN r.id
    CASE Circle(c)
      RETURN c.id
    CASE Line(l)
      RETURN l.id
    CASE Arc(a)
      RETURN a.id
    CASE Polygon(p)
      RETURN p.id
    CASE Text(t)
      RETURN t.id
    CASE Ellipse(e)
      RETURN e.id
    CASE Group(g)
      RETURN g.id
    CASE Picture(pic)
      RETURN pic.id
    CASE PolyLine(pl)
      RETURN pl.id
    CASE PolyCurve(pc)
      RETURN pc.id
    CASE CurvePolygon(cp)
      RETURN cp.id
    ' An `AutoPaint` has no `id` of its own: it reports its shape's.
    CASE AutoPaint(a)
      RETURN __canvas_itemId(__canvas_autoItem(a))
    ' plan-150-A: a particle system has no `id` -- nothing about it is ever hit.
    CASE ParticleSystem(ps)
      RETURN ""
    ' plan-173: nor is a tile; it has no `id` to report.
    CASE Tile(tile)
      RETURN ""
  END MATCH
END FUNC

' The topmost entry of the calling thread's draw list under a surface point, or -1.
' Last entry first: later entries are drawn over earlier ones.
FUNC __canvas_hitAt(x AS Float, y AS Float) AS Integer
  MUT i AS Integer = len(__CANVAS_HIT_OFFSETS) - 1
  WHILE i >= 0
    LET offset AS Integer = collections::getOr(__CANVAS_HIT_OFFSETS, i, 0)
    LET mat AS Integer = collections::getOr(__CANVAS_DRAW_MAT, i, 0)
    LET gdx AS Float = collections::getOr(__CANVAS_DRAW_DX, i, 0.0)
    LET gdy AS Float = collections::getOr(__CANVAS_DRAW_DY, i, 0.0)
    IF __canvas_hitEntry(offset, mat, gdx, gdy, x, y) THEN
      RETURN i
    END IF
    i = i - 1
  END WHILE
  RETURN 0 - 1
END FUNC

' A record already in this thread's cache under `hash`, or -1.
FUNC __canvas_hitCached(hash AS Integer) AS Integer
  LET slot AS Integer = canvas::geoFind(hash)
  IF slot < 0 THEN
    RETURN 0 - 1
  END IF
  __canvas_geoStamp(slot)
  RETURN collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
END FUNC

' A `Text` as a hit test needs it and nothing more -- its paint (for the transform and
' the clip) and its layout box, and no glyph: the run is shaped for its advances and
' never rasterised. A stroked run is the same record; the outlines the renderer lowers
' it to are no part of where it is hit. A run with no glyph is no record, as it is for
' the renderer (`__canvas_glyphRunHeader`).
FUNC __canvas_textBoxHeader(t AS Text) AS List OF Float
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text)
  IF len(shaped) < 3 OR NOT __canvas_runDrawable(shaped, scales) THEN
    RETURN __canvas_emptyHeader()
  END IF
  LET x0 AS Float = __canvas_textX(t)
  LET baseline AS Float = __canvas_textY(t)
  LET pen AS Float = __canvas_runPen(shaped, scales, x0)
  LET line AS List OF Float = __canvas_runLine(handles, scales, shaped)
  LET y0 AS Float = baseline - collections::getOr(line, 0, 0.0)
  LET y1 AS Float = baseline + collections::getOr(line, 1, 0.0)
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_TEXT))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = __canvas_paintHeader(out, t.paint, __canvas_textTransform(t), x0, baseline)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX, x0)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 1, y0)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 2, pen)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 3, y1)
  RETURN __canvas_boundsHeader(out, x0, y0, pen, y1, 0.0, 0.0, 0.0)
END FUNC

' The record a hit test reads for one item. Every kind but one is the record the
' renderer draws from (`__canvas_geometryFor`). A `Text` is its layout box: what the
' renderer's record adds to that -- every glyph of the run rasterised -- no hit test
' reads, and this thread draws nothing. A `Picture` never arrives: it is hit as its
' shape (`__canvas_hitAppend`).
FUNC __canvas_hitGeometry(item AS DrawItem, hash AS Integer) AS Integer
  MATCH item
    CASE Text(t)
      LET found AS Integer = __canvas_hitCached(hash)
      IF found >= 0 THEN
        RETURN found
      END IF
      LET header AS List OF Float = __canvas_textBoxHeader(t)
      LET built AS List OF List OF Float = __canvas_withClipEdges(header, [], item)
      LET none AS List OF Float = []
      RETURN canvas::geoInsert(hash, collections::getOr(built, 0, header), collections::getOr(built, 1, none))
    ' An `AutoPaint` is hit where its shape is, by the shape's own paint.
    CASE AutoPaint(a)
      RETURN __canvas_hitGeometry(__canvas_autoItem(a), hash)
    CASE ELSE
      RETURN __canvas_geometryFor(item, hash)
  END MATCH
END FUNC

' One draw entry, or a whole group's worth: `__canvas_appendDraw`'s walk, with the id
' path carried down beside the composed affine. A group node passes its children the
' path joined with its own `id`; a leaf records the path joined with its own. The path
' belongs to the ENTRY -- one item reached through two nodes is two entries with two
' paths, and the group's stored items are not renamed.
FUNC __canvas_hitAppend(item AS DrawItem, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS String) AS Nothing
  MATCH item
    CASE Group(g)
      ' The depth stop and the no-area stop are `__canvas_appendDraw`'s: what it does
      ' not draw is not there to be hit.
      IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN
        RETURN
      END IF
      LET childMat AS Integer = __canvas_composeGroup(gdx, gdy, mat, g.position, g.transform)
      IF childMat < 0 THEN
        RETURN
      END IF
      LET childX AS Float = __CANVAS_COMPOSED_X
      LET childY AS Float = __CANVAS_COMPOSED_Y
      LET childPath AS String = __canvas_joinId(path, g.id)
      FOR EACH child IN canvas::groupItems(canvas::groupResolve(g.name))
        __canvas_hitAppend(child, childX, childY, childMat, depth + 1, childPath)
      NEXT
    ' plan-172-L: a `Picture` is hit as its shape, and its own `id` goes in front of
    ' the shape's -- as a group's goes in front of its children's. A picture that draws
    ' nothing answers a group that names nothing, and appends nothing.
    CASE Picture(pic)
      __canvas_hitAppend(__canvas_pictureHitItem(pic), gdx, gdy, mat, depth, __canvas_joinId(path, pic.id))
    ' plan-150-A: a particle is never under the mouse. A system appends nothing, so it
    ' is not reported by `overID` and hides nothing beneath it -- and this thread never
    ' runs the particle model.
    CASE ParticleSystem(ps)
      RETURN
    ' plan-173: a tile is never under the mouse either, and hides nothing beneath it --
    ' a map is a backdrop, and the items over it are what `overID` names.
    CASE Tile(tile)
      RETURN
    CASE ELSE
      LET offset AS Integer = __canvas_hitGeometry(item, __canvas_hashItem(item))
      LET id AS String = __canvas_joinId(path, __canvas_itemId(item))
      __CANVAS_HIT_OFFSETS = collections::append(__CANVAS_HIT_OFFSETS, offset)
      __CANVAS_DRAW_DX = collections::append(__CANVAS_DRAW_DX, gdx)
      __CANVAS_DRAW_DY = collections::append(__CANVAS_DRAW_DY, gdy)
      __CANVAS_DRAW_MAT = collections::append(__CANVAS_DRAW_MAT, mat)
      __CANVAS_HIT_IDS = collections::append(__CANVAS_HIT_IDS, id)
      IF id <> "" THEN
        __CANVAS_HIT_NAMED = TRUE
      END IF
  END MATCH
END FUNC

' Does anything a list reaches carry an `id` -- an item, a group node, or an item of a
' group it names, at any depth? Asked only of a scene whose own items have none
' (`__CANVAS_SCENE_IDS` = 2), before any geometry is built for it.
FUNC __canvas_reachesId(items AS List OF DrawItem, depth AS Integer) AS Boolean
  IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN
    RETURN FALSE
  END IF
  FOR EACH item IN items
    IF __canvas_itemId(item) <> "" THEN
      RETURN TRUE
    END IF
    MATCH item
      CASE Group(g)
        IF __canvas_reachesId(canvas::groupItems(canvas::groupResolve(g.name)), depth + 1) THEN
          RETURN TRUE
        END IF
      ' plan-172-L: a picture's shape has an `id` of its own.
      CASE Picture(pic)
        IF __canvas_pictureShapeId(pic) <> "" THEN
          RETURN TRUE
        END IF
      CASE ELSE
        LET leaf AS Integer = 0
    END MATCH
  NEXT
  RETURN FALSE
END FUNC

' Build this thread's draw list from the scene it last presented: the installed items,
' then every layer's, the order they are drawn in.
FUNC __canvas_hitBuild() AS Nothing
  __CANVAS_HIT_REV = __CANVAS_SCENE_REV
  __CANVAS_HIT_OFFSETS = []
  __CANVAS_HIT_IDS = []
  __CANVAS_HIT_NAMED = FALSE
  __CANVAS_DRAW_DX = []
  __CANVAS_DRAW_DY = []
  __CANVAS_DRAW_MAT = []
  __CANVAS_GROUP_ROWS = []
  LET items AS List OF DrawItem = canvas::installedItems()
  LET layers AS List OF DrawLayer = canvas::installedLayers()
  IF __CANVAS_SCENE_IDS = 2 THEN
    MUT reached AS Boolean = __canvas_reachesId(items, 0)
    FOR EACH layer IN layers
      IF __canvas_reachesId(layer.items, 0) THEN
        reached = TRUE
      END IF
    NEXT
    IF NOT reached THEN
      RETURN
    END IF
  END IF
  ' The one point where no offset of this thread's cache is live: the last list's are
  ' dropped above and this one's do not exist yet (`canvas::geoBeginFrame`).
  canvas::geoBeginFrame()
  FOR EACH item IN items
    __canvas_hitAppend(item, 0.0, 0.0, 0, 0, "")
  NEXT
  FOR EACH layer IN layers
    FOR EACH item IN layer.items
      __canvas_hitAppend(item, 0.0, 0.0, 0, 0, "")
    NEXT
  NEXT
END FUNC

' The id path of the topmost item under a surface point in the scene this thread last
' presented, or "".
'
' The list is built at most once per presented scene, and only here -- when an event is
' actually being returned. A scene in which nothing has an `id` (`__CANVAS_SCENE_IDS`
' = 0, recorded by `present`) never builds one.
FUNC __canvas_overId(x AS Float, y AS Float) AS String
  IF __CANVAS_SCENE_IDS = 0 THEN
    RETURN ""
  END IF
  IF __CANVAS_HIT_REV <> __CANVAS_SCENE_REV THEN
    __canvas_hitBuild()
  END IF
  IF NOT __CANVAS_HIT_NAMED THEN
    RETURN ""
  END IF
  LET at AS Integer = __canvas_hitAt(x, y)
  IF at < 0 THEN
    RETURN ""
  END IF
  RETURN collections::getOr(__CANVAS_HIT_IDS, at, "")
END FUNC

' `canvas::pollMouse`: the native poll's event, with the id path under it.
'
' The native event is already whole -- `overID` empty -- so it is handed straight back
' whenever that is the answer: an idle poll, a scene with no `id` in it, a press over
' nothing. Only an event that is over something named pays for a second record.
' `position` is a whole pixel, so the point tested is that pixel's centre -- the point
' the rasteriser evaluates the same shapes at.
FUNC __canvas_pollMouse() AS MouseEvent
  LET event AS MouseEvent = canvas::pollMouseRaw()
  IF event.kind = MouseKind.None OR __CANVAS_SCENE_IDS = 0 THEN
    RETURN event
  END IF
  LET over AS String = __canvas_overId(event.position.x + 0.5, event.position.y + 0.5)
  IF over = "" THEN
    RETURN event
  END IF
  RETURN WITH event { overID := over }
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "canvas_hit_region",
        gate: HelperGate::WhenUsedOrConstructed(
            &["pollMouse"],
            super::helper_autopaint::CONSTRUCTED,
        ),
        body: Some(REGION),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "canvas_hit",
        gate: HelperGate::WhenUsed(&["pollMouse"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}

#[cfg(test)]
mod tests {
    use super::{BODY, REGION};

    /// The slack an open kind is given is stated twice in the chunk — the bounds
    /// reject and the stroke test — and a bounds reject narrower than the test would
    /// make the last pixels of the slack unreachable without failing anything.
    #[test]
    fn canvas_hit_the_bounds_reject_is_as_wide_as_the_open_kind_slack() {
        assert_eq!(REGION.matches("- 3.0 OR").count(), 2);
        assert_eq!(REGION.matches("+ 3.0 THEN").count(), 2);
        assert_eq!(REGION.matches("d <= 3.0 * scale").count(), 1);
    }

    /// The path separator is the language's namespace separator, spelled once.
    #[test]
    fn canvas_hit_the_id_path_separator_is_spelled_once() {
        assert_eq!(BODY.matches("\"::\"").count(), 1);
    }
}
