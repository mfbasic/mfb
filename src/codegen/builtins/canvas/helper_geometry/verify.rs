//! plan-176-R: `helper_geometry`'s MFBASIC helper text — the geometry verifier (`GEO_VERIFY`,
//! `GEO_VERIFY_DEBUG`). Registration lives in `builtins/canvas/helper_geometry/mod.rs`.

/// bug-686: the check on `canvas::geoBuild`, the native geometry builder — normal-build
/// body.
///
/// A normal build has nothing to check against and nowhere to report, so the hook is an
/// empty `FUNC … AS Nothing`, like `__canvas_phaseMark`.
#[rustfmt::skip]
pub(super) const GEO_VERIFY: &str = r#"FUNC __canvas_geoVerify(item AS DrawItem, native AS List OF Float) AS Nothing
END FUNC

FUNC __canvas_geoVerifyFrame(pending AS Integer) AS Nothing
END FUNC

FUNC __canvas_drawsVerify() AS Nothing
END FUNC"#;

/// [`GEO_VERIFY`] for a `--debug` build.
///
/// Reports every record the native builder produced (`geoNative=` on the stats line,
/// `__CANVAS_GEO_NATIVE_BUILDS`), and while `MFB_CANVAS_GEO_VERIFY=1` builds the same
/// item's record with the MFBASIC builders and counts the ones that differ in any bit
/// (`geoVerified=`, `geoVerifyMismatches=`) — both the records `__canvas_geometryFor`
/// built through `canvas::geoBuild` and the ones `canvas::sceneResolve` built in its
/// frame pass (`__canvas_geoVerifyFrame`), plus the draw hashes that pass folded, and
/// the draw list `canvas::sceneDrawsFlat` laid out (`__canvas_drawsVerify`, counted in
/// `drawsVerified=`; its mismatches count in `geoVerifyMismatches=` too). Records are
/// compared with `canvas::geoSame`, which compares the 64-bit patterns: a float `=`
/// would call `-0.0` and `0.0` the same, and the software rasteriser's goldens would not. The variable is read once and cached (0 unresolved,
/// 1 off, 2 on), like `__canvas_dumpRequested`.
///
/// `__canvas_geoReference` is `__canvas_geometryFor`'s own arms for the five kinds the
/// native builder takes: the header followed by the tail, as the cache appends them.
#[rustfmt::skip]
pub(super) const GEO_VERIFY_DEBUG: &str = r#"MUT __CANVAS_GEO_VERIFY_MODE AS Integer = 0
MUT __CANVAS_GEO_VERIFIED AS Integer = 0
MUT __CANVAS_DRAWS_VERIFIED AS Integer = 0
MUT __CANVAS_GEO_MISMATCHES AS Integer = 0
MUT __CANVAS_GEO_RESOLVED_CHECKED AS Integer = 0

' The record `__canvas_geometryFor` would build for `item`, from its own arms, for every
' kind. `offset` is where the record under test starts, or -1: a glyph run's entries are
' glyph-cache INDICES, which only the cache can assign, so they are taken from the
' record under test -- each checked against the glyph key it must name -- and everything
' else about the run (its length, pens, baseline, header) is rebuilt.
FUNC __canvas_geoReference(item AS DrawItem, offset AS Integer) AS List OF Float
  MUT tail AS List OF Float = []
  MUT header AS List OF Float = []
  MATCH item
    CASE Rectangle(r)
      LET radius AS Float = __canvas_rectRadius(r)
      IF radius >= 0.0 THEN
        header = __canvas_rectItem(r, radius)
        LET joins AS List OF Float = __canvas_rectJoins(header, r.paint)
        LET dashes AS List OF Float = __canvas_rectDashes(header, r.paint)
        tail = __canvas_appendGradientTail(collections::append(joins, dashes), r.paint)
        header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(r.paint), r.paint)), dashes, r.paint)
      ELSE
        ' plan-172-E: corners that are not one circular radius. The outline is a polygon
        ' ring, so the rest is the polygon's own arm: edges, joins, dashes, stops.
        LET ring AS List OF List OF vector::Float2 = __canvas_rectRing(r)
        LET edges AS List OF Float = __canvas_polygonEdges(ring)
        LET joins AS List OF Float = __canvas_joinTail(edges, __canvas_ringStarts(ring), TRUE, __canvas_strokeHalf(r.paint), r.paint)
        header = __canvas_polygonHeader(ring, FillRule.EvenOdd, r.paint, __canvas_placement(r.position, r.transform), __canvas_buildX(r.position, r.transform), __canvas_buildY(r.position, r.transform))
        LET dashes AS List OF Float = __canvas_edgeDashes(header, r.paint, __canvas_strokeHalf(r.paint), edges, __canvas_ringStarts(ring), TRUE)
        IF len(edges) > 0 THEN
          tail = __canvas_appendGradientTail(collections::append(collections::append(edges, joins), dashes), r.paint)
        END IF
        header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(r.paint), r.paint)), dashes, r.paint)
      END IF
    CASE Circle(c)
      header = __canvas_circleItem(c)
      LET dashes AS List OF Float = __canvas_roundDashes(header, c.paint)
      tail = __canvas_appendGradientTail(dashes, c.paint)
      header = __canvas_withDashes(header, dashes, c.paint)
    CASE Line(l)
      header = __canvas_lineHeader(l)
      LET dashes AS List OF Float = __canvas_lineDashes(header, l)
      tail = __canvas_strokeRampTail(dashes, l.paint)
      header = __canvas_withDashes(header, dashes, l.paint)
    CASE Polygon(p)
      ' plan-172-F: the points are offsets from `position`. The joins and the dashes are
      ' computed from `edges`, never from the item's points, so they follow.
      LET placed AS List OF List OF vector::Float2 = __canvas_placedRings(p.contours, p.position, p.transform)
      LET edges AS List OF Float = __canvas_polygonEdges(placed)
      LET joins AS List OF Float = __canvas_joinTail(edges, __canvas_ringStarts(placed), TRUE, __canvas_strokeHalf(p.paint), p.paint)
      header = __canvas_polygonHeader(placed, p.rule, p.paint, __canvas_placement(p.position, p.transform), __canvas_buildX(p.position, p.transform), __canvas_buildY(p.position, p.transform))
      LET dashes AS List OF Float = __canvas_edgeDashes(header, p.paint, __canvas_strokeHalf(p.paint), edges, __canvas_ringStarts(placed), TRUE)
      IF len(edges) > 0 THEN
        tail = __canvas_appendGradientTail(collections::append(collections::append(edges, joins), dashes), p.paint)
      END IF
      header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(p.paint), p.paint)), dashes, p.paint)
    CASE Arc(a)
      header = __canvas_arcHeader(a)
      LET dashes AS List OF Float = __canvas_arcDashes(header, a)
      tail = __canvas_strokeRampTail(dashes, a.paint)
      header = __canvas_withDashes(header, dashes, a.paint)
    ' plan-172-L: a picture's records have ONE builder (`helper_picture.rs`) -- the
    ' native one declines a picture -- so there is no second answer to hold one against,
    ' and the record under test is its own reference. What a shaped picture's outline is
    ' built from IS checked, as the item it is, where it is built
    ' (`__canvas_geoVerify`).
    CASE Picture(pic)
      IF offset < 0 THEN
        RETURN __canvas_emptyHeader()
      END IF
      RETURN collections::mid(__CANVAS_GEO_DATA, offset, toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 1, 0.0)))
    CASE Tile(tile)
      IF offset < 0 THEN
        RETURN __canvas_emptyHeader()
      END IF
      RETURN collections::mid(__CANVAS_GEO_DATA, offset, toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 1, 0.0)))
    CASE Ellipse(e)
      header = __canvas_ellipseHeader(e)
      LET dashes AS List OF Float = __canvas_ellipseDashes(header, e)
      tail = __canvas_appendGradientTail(dashes, e.paint)
      header = __canvas_withDashes(header, dashes, e.paint)
    CASE Text(t)
      IF __canvas_strokeHalf(t.paint) > 0.0 THEN
        LET edges AS List OF Float = __canvas_textEdges(t)
        LET joins AS List OF Float = __canvas_joinTail(edges, __CANVAS_RING_MARKS, TRUE, __canvas_strokeHalf(t.paint), t.paint)
        header = __canvas_textHeader(t, edges)
        LET dashes AS List OF Float = __canvas_edgeDashes(header, t.paint, __canvas_strokeHalf(t.paint), edges, __CANVAS_RING_MARKS, TRUE)
        tail = collections::append(collections::append(edges, joins), dashes)
        header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(t.paint), t.paint)), dashes, t.paint)
      ELSE
        tail = __canvas_geoRunReference(t, offset)
        header = __canvas_textHeader(t, tail)
      END IF
      ' plan-172-K: the layout box `__canvas_geometryFor` writes into a text's record
      ' in a program with an `AutoPaint`.
      header = __canvas_autoTextBox(header, t)
    CASE Group(g)
      header = __canvas_emptyHeader()
    ' plan-172-K: the record the frame holds for an `AutoPaint`, before its resolve, is
    ' its shape's with the shape's own paint -- so that is its reference. Found by
    ' `scripts/test-canvas-gpu-rows.sh`, whose first `AutoPaint` rows counted three
    ' mismatches: this arm answered the empty header, and the two labels had no box.
    CASE AutoPaint(a)
      RETURN __canvas_geoReference(__canvas_autoItem(a), offset)
    ' plan-150-A: expanded before any record is built for it, like a group.
    CASE ParticleSystem(ps)
      header = __canvas_emptyHeader()
    CASE PolyLine(pl)
      LET moved AS PolyLine = __canvas_polylinePlaced(pl)
      header = __canvas_polylineHeader(moved)
      IF toInt(collections::getOr(header, 0, 0.0)) = __CANVAS_GEO_POLYLINE THEN
        tail = __canvas_polylineTail(moved)
        header = __canvas_withDashes(__canvas_withJoins(header, __canvas_polylineJoins(moved), __canvas_joinReach(__canvas_polylineHalf(moved), moved.paint)), __canvas_polylineDashes(moved), moved.paint)
      END IF
    ' plan-164-B: the two curve shapes, one builder (`helper_curves.rs`): a chain is
    ' one open contour.
    CASE PolyCurve(pc)
      LET built AS List OF List OF Float = __canvas_polyCurveRecord(pc)
      header = collections::getOr(built, 0, header)
      tail = collections::getOr(built, 1, tail)
    CASE CurvePolygon(cp)
      LET built AS List OF List OF Float = __canvas_curvePolygonRecord(cp)
      header = collections::getOr(built, 0, header)
      tail = collections::getOr(built, 1, tail)
  END MATCH
  LET clipped AS List OF List OF Float = __canvas_withClipEdges(header, tail, item)
  MUT out AS List OF Float = collections::getOr(clipped, 0, header)
  out = collections::append(out, collections::getOr(clipped, 1, tail))
  RETURN out
END FUNC

' `__canvas_textGlyphRun` without touching the glyph cache: each glyph's entry is read
' from the record at `offset` and kept only if the cache entry it names holds this
' glyph's key (else -2, which no run holds).
FUNC __canvas_geoRunReference(t AS Text, offset AS Integer) AS List OF Float
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text, t.letterSpacing)
  IF NOT __canvas_runDrawable(shaped, scales) THEN
    RETURN []
  END IF
  MUT run AS List OF Float = []
  MUT pen AS Float = __canvas_textX(t)
  ' The record's triples are only the drawn glyphs', so the stored one is counted apart.
  MUT at AS Integer = 0
  MUT c AS Integer = 0
  WHILE c * __CANVAS_RUN_STRIDE < len(shaped)
    LET which AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0)
    LET scale AS Float = collections::getOr(scales, which, 0.0)
    IF scale > 0.0 THEN
      LET gid AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GLYPH, 0)
      MUT entry AS Integer = 0 - 2
      IF offset >= 0 THEN
        LET stored AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + at * 3, 0.0 - 2.0))
        IF stored >= 0 AND collections::getOr(__CANVAS_GLYPH_KEYS, stored, 0 - 1) = __canvas_glyphKey(collections::getOr(handles, which, 0), __canvas_sizeQ(t.fontSize), gid) THEN
          entry = stored
        END IF
      END IF
      run = collections::append(run, toFloat(entry))
      run = collections::append(run, toFloat(toInt(pen + 0.5)))
      run = collections::append(run, toFloat(toInt(__canvas_textY(t) + 0.5)))
      at = at + 1
    END IF
    pen = pen + toFloat(collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_ADVANCE, 0)) * scale
    IF t.letterSpacing <> 0.0 AND collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GAP, 0) = 1 THEN
      pen = pen + t.letterSpacing
    END IF
    c = c + 1
  END WHILE
  RETURN run
END FUNC

FUNC __canvas_geoVerifying() AS Boolean
  IF __CANVAS_GEO_VERIFY_MODE = 0 THEN
    __CANVAS_GEO_VERIFY_MODE = 1
    IF os::getEnvOr("MFB_CANVAS_GEO_VERIFY", "") = "1" THEN
      __CANVAS_GEO_VERIFY_MODE = 2
    END IF
  END IF
  RETURN __CANVAS_GEO_VERIFY_MODE = 2
END FUNC

FUNC __canvas_geoVerify(item AS DrawItem, native AS List OF Float) AS Nothing
  IF __canvas_geoVerifying() THEN
    __CANVAS_GEO_VERIFIED = __CANVAS_GEO_VERIFIED + 1
    IF NOT canvas::geoSame(native, __canvas_geoReference(item, 0 - 1)) THEN
      __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
    END IF
  END IF
END FUNC

' The frame `canvas::sceneResolve` just laid out: every record it built natively,
' against the MFBASIC builders, and -- when no index went to MFBASIC, so the draw
' entries are the scene's one for one -- every draw hash it folded, against
' `__canvas_hashFloat`.
FUNC __canvas_geoVerifyFrame(pending AS Integer) AS Nothing
  IF NOT __canvas_geoVerifying() THEN
    RETURN
  END IF
  IF len(__CANVAS_GEO_BUILT) > 0 THEN
    LET items AS List OF DrawItem = __canvas_frameItems()
    LET none AS DrawItem = __canvas_noItem()
    MUT k AS Integer = 0
    WHILE k + 1 < len(__CANVAS_GEO_BUILT)
      LET index AS Integer = collections::getOr(__CANVAS_GEO_BUILT, k, 0)
      LET slot AS Integer = collections::getOr(__CANVAS_GEO_BUILT, k + 1, 0)
      LET offset AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
      LET count AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 2, 0)
      LET record AS List OF Float = collections::mid(__CANVAS_GEO_DATA, offset, count)
      __CANVAS_GEO_VERIFIED = __CANVAS_GEO_VERIFIED + 1
      IF NOT canvas::geoSame(record, __canvas_geoReference(collections::getOr(items, index, none), offset)) THEN
        __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
      END IF
      k = k + 2
    END WHILE
  END IF
  ' Every index the frame resolved, whatever its kind and however it was resolved --
  ' a hit included: the record at its offset must be the one the item the frame draws
  ' at that index builds. A hit is trusted on its hash alone, so this is the check that
  ' the hash and the item came from the same scene.
  LET top AS Integer = len(__CANVAS_TOP_OFFSETS)
  IF top > 0 THEN
    LET drawn AS List OF DrawItem = __canvas_frameItems()
    LET absent AS DrawItem = __canvas_noItem()
    MUT r AS Integer = 0
    WHILE r < top
      LET at AS Integer = collections::getOr(__CANVAS_TOP_OFFSETS, r, 0 - 1)
      IF at >= 0 THEN
        LET length AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, at + 1, 0.0))
        __CANVAS_GEO_RESOLVED_CHECKED = __CANVAS_GEO_RESOLVED_CHECKED + 1
        IF NOT canvas::geoSame(collections::mid(__CANVAS_GEO_DATA, at, length), __canvas_geoReference(collections::getOr(drawn, r, absent), at)) THEN
          __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
        END IF
      END IF
      r = r + 1
    END WHILE
  END IF
  IF pending = 0 THEN
    LET hashes AS List OF Integer = __CANVAS_FRAME_HASHES
    MUT i AS Integer = 0
    WHILE i < len(__CANVAS_DRAW_HASHES) AND i < len(hashes)
      LET folded AS Integer = __canvas_hashFloat(__canvas_hashFloat(collections::getOr(hashes, i, 0), 0.0), 0.0)
      IF collections::getOr(__CANVAS_DRAW_HASHES, i, 0) <> folded THEN
        __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
      END IF
      i = i + 1
    END WHILE
  END IF
END FUNC

' The draw list `canvas::sceneDrawsFlat` laid out, against the MFBASIC walk it replaces
' on a frame with no group -- which is re-run here and left in place.
FUNC __canvas_drawsVerify() AS Nothing
  IF NOT __canvas_geoVerifying() THEN
    RETURN
  END IF
  LET draws AS List OF Integer = __CANVAS_DRAWS
  LET blocks AS List OF Integer = __CANVAS_DRAW_BLOCKS
  LET inst AS List OF Integer = __CANVAS_DRAW_INST
  LET nextInst AS Integer = __CANVAS_DRAW_NEXT_INST
  LET walked AS List OF Integer = __canvas_sceneDrawsWalk()
  __CANVAS_DRAWS_VERIFIED = __CANVAS_DRAWS_VERIFIED + 1
  IF NOT (__canvas_intListEquals(draws, walked) AND __canvas_intListEquals(blocks, __CANVAS_DRAW_BLOCKS) AND __canvas_intListEquals(inst, __CANVAS_DRAW_INST) AND nextInst = __CANVAS_DRAW_NEXT_INST) THEN
    __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
  END IF
END FUNC

FUNC __canvas_geoVerifyText() AS String
  RETURN " geoNative=" & toString(__CANVAS_GEO_NATIVE_BUILDS) & " geoVerified=" & toString(__CANVAS_GEO_VERIFIED) & " geoVerifyMismatches=" & toString(__CANVAS_GEO_MISMATCHES) & " drawsVerified=" & toString(__CANVAS_DRAWS_VERIFIED) & " geoResolvedChecked=" & toString(__CANVAS_GEO_RESOLVED_CHECKED) & " snapshotRehashed=" & toString(__CANVAS_SNAP_REHASHED)
END FUNC"#;
