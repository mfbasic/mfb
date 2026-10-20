//! plan-176-S: `helper_render`'s MFBASIC text for presenting — the direct present and closing retired surfaces. Registration lives in
//! `builtins/canvas/helper_render/mod.rs`.

/// bug-686 Phase 4: the two `--debug`-only facts `__canvas_renderMetal` needs about a
/// directly presented frame — normal-build bodies.
///
/// A normal build has no frame dump (plan-130-E), so nothing ever needs a GPU frame's
/// pixels on the CPU for it, and nothing writes a stats line. Both bodies are therefore
/// the answer, not a stub: `FALSE`, and nothing to do.
#[rustfmt::skip]
pub(super) const DIRECT_PRESENT: &str =
r#"FUNC __canvas_dumpRequested() AS Boolean
  RETURN FALSE
END FUNC

FUNC __canvas_presentedDirect() AS Nothing
END FUNC"#;

/// [`DIRECT_PRESENT`] for a `--debug` build.
///
/// `MFB_CANVAS_DUMP` names a file every frame's RGBA is written to, so while it is set
/// the frame must be read back — the direct present has no CPU pixels to write. Read
/// once and cached, like `__canvas_damageEnabled`, rather than a `getenv` per frame
/// (0 unresolved, 1 off, 2 on).
///
/// A directly presented frame never reaches `__canvas_presentSurface`, which is where
/// the stats line is written for every other rendered frame, so it writes it here —
/// after `__CANVAS_GPU_FRAMES` is bumped, for the reason `__canvas_renderMetal` gives.
///
/// `__CANVAS_DIRECT_FRAMES` is the stats line's `directFrames=`: how many of the
/// `gpuFrames=` never came back to the CPU. `gpuFrames=` alone cannot tell the two
/// GPU paths apart — both draw the same picture — and the real-window test in
/// `scripts/test-macapp.sh` needs to know which one it captured.
#[rustfmt::skip]
pub(super) const DIRECT_PRESENT_DEBUG: &str =
r#"MUT __CANVAS_DUMP_MODE AS Integer = 0
MUT __CANVAS_DIRECT_FRAMES AS Integer = 0

FUNC __canvas_dumpRequested() AS Boolean
  IF __CANVAS_DUMP_MODE = 0 THEN
    __CANVAS_DUMP_MODE = 1
    IF len(os::getEnvOr("MFB_CANVAS_DUMP", "")) > 0 THEN
      __CANVAS_DUMP_MODE = 2
    END IF
  END IF
  RETURN __CANVAS_DUMP_MODE = 2
END FUNC

FUNC __canvas_presentedDirect() AS Nothing
  __CANVAS_DIRECT_FRAMES = __CANVAS_DIRECT_FRAMES + 1
  __canvas_writeStats()
END FUNC"#;

/// plan-116-J: close the resources a retired group buffer owned.
///
/// **Not "close what the retired buffer named" — close what no LIVE buffer names**, and
/// the difference is the commonest canvas program there is (**J14**). One long-lived font
/// and a group rebuilt each frame names the same font in the retired buffer *and* in the
/// buffer that replaced it; closing on the plain rule makes its text vanish one frame
/// later, silently, because `fontHandle` then answers `0` and `0` is "no such object".
///
/// Identity is compared through `canvas::imageHandle`/`canvas::fontHandle`, which return
/// the backend id as an `Integer`. That is only possible because plan-116-I added them —
/// two aliases of one resource cannot be compared as `RES` values — and it is why their
/// read order matters here too: they test the closed flag **before** loading the handle,
/// so a concurrent destroy cannot yield a stale non-zero id that would keep a resource
/// alive that nothing names.
///
/// `0` is skipped on both sides: an already-closed resource has nothing to close, and it
/// must not match a live one either.
pub(super) const CLOSE_RETIRED: &str = r#"FUNC __canvas_closeRetired(gone AS List OF DrawItem, scene AS List OF DrawItem) AS Nothing
  IF len(gone) = 0 THEN
    RETURN
  END IF
  FOR EACH item IN gone
    MATCH item
      CASE Picture(p)
        LET ih AS Integer = canvas::imageHandle(p.image)
        IF ih <> 0 AND NOT __canvas_anythingNamesImage(scene, ih) THEN
          canvas::destroyImage(p.image)
        END IF
        ' plan-172-L: and the font of a `Text` it has for a shape.
        __canvas_pictureCloseFont(p, scene)
      CASE Tile(t)
        LET ih AS Integer = canvas::imageHandle(t.image)
        IF ih <> 0 AND NOT __canvas_anythingNamesImage(scene, ih) THEN
          canvas::destroyImage(t.image)
        END IF
      CASE Text(t)
        __canvas_closeTextFonts(t, scene)
      ' plan-172-K: an `AutoPaint` around a `Text` names that text's fonts.
      CASE AutoPaint(a)
        MATCH a.shape
          CASE Text(t)
            __canvas_closeTextFonts(t, scene)
          CASE ELSE
        END MATCH
      ' plan-150-A: a particle system names the font of a `Text` it has for a template
      ' or for an emitter.
      CASE ParticleSystem(ps)
        __canvas_particleCloseFonts(ps, scene)
      CASE ELSE
    END MATCH
  NEXT
END FUNC

' Everything still live: the scene about to be published, and EVERY group's live items.
'
' Not just the replacing slot's. Two other holders reach the same resource and both are
' reachable from ordinary programs: another group naming it, and the scene naming it
' directly. `present` does not take ownership, so a Picture built before the setGroup
' reaches the scene with nothing for the move checker to object to -- and closing it there
' makes the scene's item draw nothing, silently, one frame later.
'
' Every slot rather than the ones the scene references: an unreferenced group is not
' drawn today and may be drawn tomorrow, so its items are live regardless.
FUNC __canvas_anythingNamesImage(scene AS List OF DrawItem, handle AS Integer) AS Boolean
  IF __canvas_listNamesImage(scene, handle) THEN
    RETURN TRUE
  END IF
  MUT i AS Integer = 0
  LET slots AS Integer = canvas::groupSlots()
  WHILE i < slots
    IF __canvas_listNamesImage(canvas::groupItems(i), handle) THEN
      RETURN TRUE
    END IF
    i = i + 1
  END WHILE
  RETURN FALSE
END FUNC

FUNC __canvas_anythingNamesFont(scene AS List OF DrawItem, handle AS Integer) AS Boolean
  IF __canvas_listNamesFont(scene, handle) THEN
    RETURN TRUE
  END IF
  MUT i AS Integer = 0
  LET slots AS Integer = canvas::groupSlots()
  WHILE i < slots
    IF __canvas_listNamesFont(canvas::groupItems(i), handle) THEN
      RETURN TRUE
    END IF
    i = i + 1
  END WHILE
  RETURN FALSE
END FUNC

FUNC __canvas_listNamesImage(live AS List OF DrawItem, handle AS Integer) AS Boolean
  FOR EACH item IN live
    MATCH item
      CASE Picture(p)
        IF canvas::imageHandle(p.image) = handle THEN
          RETURN TRUE
        END IF
      CASE Tile(t)
        IF canvas::imageHandle(t.image) = handle THEN
          RETURN TRUE
        END IF
      CASE ELSE
    END MATCH
  NEXT
  RETURN FALSE
END FUNC

FUNC __canvas_listNamesFont(live AS List OF DrawItem, handle AS Integer) AS Boolean
  FOR EACH item IN live
    MATCH item
      CASE Text(t)
        IF __canvas_textNamesFont(t, handle) THEN
          RETURN TRUE
        END IF
      ' plan-172-K: and so does an `AutoPaint` around one.
      CASE AutoPaint(a)
        MATCH a.shape
          CASE Text(t)
            IF __canvas_textNamesFont(t, handle) THEN
              RETURN TRUE
            END IF
          CASE ELSE
        END MATCH
      ' plan-172-L: and a `Picture` whose shape is one.
      CASE Picture(p)
        IF __canvas_pictureNamesFont(p, handle) THEN
          RETURN TRUE
        END IF
      ' plan-150-A: and a particle system whose template or emitter is one.
      CASE ParticleSystem(ps)
        IF __canvas_particleNamesFont(ps, handle) THEN
          RETURN TRUE
        END IF
      CASE ELSE
    END MATCH
  NEXT
  RETURN FALSE
END FUNC"#;
