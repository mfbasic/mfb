//! `__canvas_renderScene` — the entry point `canvas::present` calls after a publish
//! that actually changed something.
//!
//! The renderer is written in MFBASIC source rather than emitted per architecture,
//! which is the same choice `json`, `regex` and `crypto` make for their algorithmic
//! cores. For a rasteriser it is doubly right: it is the **oracle** the GPU backends
//! (plan-98-E/F) are compared against, so it must produce identical pixels on every
//! target, and one source implementation gives that for free where five hand-written
//! assembly ports would each be a place for the oracle to disagree with itself.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

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
const RENDER_SCENE: &str =
r#"FUNC __canvas_renderScene() AS Nothing
  LET size AS Size = __canvas_surfaceSize()
  MUT buffer AS List OF Byte = canvas::newSurface(size.width, size.height)
  LET hashes AS List OF Integer = canvas::installedHashes()
  MUT index AS Integer = 0
  FOR EACH item IN canvas::installedItems()
    buffer = __canvas_drawOne(buffer, size.width, size.height, item, collections::getOr(hashes, index, 0))
    index = index + 1
  NEXT
  FOR EACH layer IN canvas::installedLayers()
    FOR EACH item IN layer.items
      buffer = __canvas_drawOne(buffer, size.width, size.height, item, collections::getOr(hashes, index, 0))
      index = index + 1
    NEXT
  NEXT
  __canvas_presentSurface(buffer, size.width, size.height)
END FUNC

FUNC __canvas_drawOne(surface AS List OF Byte, width AS Integer, height AS Integer, item AS DrawItem, hash AS Integer) AS List OF Byte
  LET offset AS Integer = __canvas_geometryFor(item, hash)
  RETURN __canvas_drawGeometry(surface, width, height, offset)
END FUNC"#;

/// The GPU renderer (plan-98-E), and the scene walk both backends share.
///
/// `__canvas_sceneOffsets` is the same traversal `__canvas_renderScene` does — flat
/// items then layers in order, one hash index across both — reduced to what a
/// backend actually needs: the cache offset of each item's geometry, in draw order.
/// Generating it costs nothing extra when the Metal path declines, because
/// `__canvas_geometryFor` is the cache and the software walk that follows hits it.
///
/// `__canvas_metalRenderable` is the honesty gate. Phase 1's fragment shader emits a
/// flat colour over the item's own extent, which is *exactly* right for a
/// square-cornered, unstroked rectangle and wrong for everything else — a circle
/// would come out as its bounding box. So the renderer draws a scene only when every
/// item is one it reproduces, and hands the rest back to the oracle. Each condition
/// disappears as Phase 2's SDF shader subsumes it.
///
/// It reads the geometry header by slot rather than through a helper because that is
/// what the header is for: a fixed 22-float layout both backends index directly
/// (`__canvas_headerFor`).
#[rustfmt::skip]
const RENDER_METAL: &str =
r#"FUNC __canvas_sceneOffsets() AS List OF Integer
  MUT offsets AS List OF Integer = []
  LET hashes AS List OF Integer = canvas::installedHashes()
  MUT index AS Integer = 0
  FOR EACH item IN canvas::installedItems()
    offsets = collections::append(offsets, __canvas_geometryFor(item, collections::getOr(hashes, index, 0)))
    index = index + 1
  NEXT
  FOR EACH layer IN canvas::installedLayers()
    FOR EACH item IN layer.items
      offsets = collections::append(offsets, __canvas_geometryFor(item, collections::getOr(hashes, index, 0)))
      index = index + 1
    NEXT
  NEXT
  RETURN offsets
END FUNC

FUNC __canvas_metalRenderable(offsets AS List OF Integer) AS Boolean
  FOR EACH offset IN offsets
    IF toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0)) <> __CANVAS_KIND_RECT THEN
      RETURN FALSE
    END IF
    IF collections::getOr(__CANVAS_GEO_DATA, offset + 6, 0.0) <> 0.0 THEN
      RETURN FALSE
    END IF
    IF collections::getOr(__CANVAS_GEO_DATA, offset + 7, 0.0) > 0.0 THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC

FUNC __canvas_renderMetal() AS Boolean
  LET offsets AS List OF Integer = __canvas_sceneOffsets()
  IF NOT __canvas_metalRenderable(offsets) THEN
    RETURN FALSE
  END IF
  LET size AS Size = __canvas_surfaceSize()
  LET buffer AS List OF Byte = canvas::newSurface(size.width, size.height)
  canvas::metalDrawScene(buffer, size.width, size.height, __CANVAS_GEO_DATA, offsets)
  __canvas_presentSurface(buffer, size.width, size.height)
  RETURN TRUE
END FUNC"#;

/// The per-item content hashes for a scene, in item order.
///
/// A layered scene flattens into one hash list in draw order, which is exactly the
/// order `__canvas_renderScene` walks — so one index serves both shapes and the
/// renderer needs no shape-specific hash lookup.
#[rustfmt::skip]
const HASH_SCENE: &str =
r#"FUNC __canvas_hashScene(items AS List OF DrawItem) AS List OF Integer
  MUT out AS List OF Integer = []
  FOR EACH item IN items
    out = collections::append(out, __canvas_hashItem(item))
  NEXT
  RETURN out
END FUNC

FUNC __canvas_hashLayers(layers AS List OF DrawLayer) AS List OF Integer
  MUT out AS List OF Integer = []
  FOR EACH layer IN layers
    FOR EACH item IN layer.items
      out = collections::append(out, __canvas_hashItem(item))
    NEXT
  NEXT
  RETURN out
END FUNC"#;

/// Start the graphics thread on the first present, and settle sync mode with it.
///
/// The guard makes this one `os::getEnvOr` per program rather than per frame, and it
/// keeps the environment read in MFBASIC — where it is already portable — instead of
/// putting a per-platform `getenv` on the spawn path.
#[rustfmt::skip]
const ENSURE_GRAPHICS: &str =
r#"MUT __CANVAS_GFX_READY AS Boolean = FALSE

FUNC __canvas_ensureGraphics() AS Nothing
  IF NOT __CANVAS_GFX_READY THEN
    canvas::setSyncMode(len(os::getEnvOr("MFB_CANVAS_SYNC", "")) > 0)
    canvas::setMetalMode(len(os::getEnvOr("MFB_CANVAS_METAL", "")) > 0)
    canvas::startGraphics()
    __CANVAS_GFX_READY = TRUE
  END IF
END FUNC"#;

/// The graphics thread's whole life: wait, render, repeat — and the renderer seam.
///
/// `__canvas_renderFrame` is the one place that will choose a renderer (plan-98-E).
/// It is deliberately a runtime branch rather than a build-time one, because the
/// choice is a runtime fact: whether a Metal device exists, and whether the program
/// asked for it (`canvas::metalAvailable` and `canvas::useMetal` answer those).
///
/// The Metal arm is taken only when all three of its conditions hold: the program
/// asked for it (`canvas::useMetal`), a pipeline exists (`canvas::metalReady`), and
/// the *scene* is one the GPU renderer draws correctly (`__canvas_renderMetal`
/// returns FALSE otherwise, and the software path runs instead).
///
/// That third condition is what keeps `MFB_CANVAS_METAL=1` honest. plan-98-E Phase 1
/// renders flat-filled quads; the SDF fragment shader that gives circles, arcs,
/// strokes and rounded corners their shape is Phase 2. A backend that drew a circle
/// as its bounding box would still *report* success, which is exactly the lie
/// Correction 3 rejected — so the renderer declines a scene it cannot draw rather
/// than drawing it wrongly, and the set it declines shrinks as the shader grows.
///
/// The software path stays the default regardless: it is the oracle the GPU path is
/// measured against, so it cannot become the thing being measured.
///
/// It never returns. The wait is a real condition wait, so a static scene costs
/// nothing — no timer, no poll, no spin (`.ai/canvas-threading.md` §4: time is
/// deliberately not a redraw trigger).
///
/// It renders the *installed* scene rather than being handed one, which is what lets
/// a repaint no `present` caused — a resize, an expose — draw the right picture.
#[rustfmt::skip]
const RENDER_LOOP: &str =
r#"FUNC __canvas_renderLoop() AS Nothing
  WHILE canvas::waitForRedraw()
    __canvas_renderFrame()
    canvas::frameDone()
  END WHILE
END FUNC

FUNC __canvas_renderFrame() AS Nothing
  IF canvas::useMetal() AND canvas::metalReady() THEN
    IF __canvas_renderMetal() THEN
      RETURN
    END IF
  END IF
  __canvas_renderScene()
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_ensureGraphics",
        ENSURE_GRAPHICS,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_renderLoop", RENDER_LOOP));
    pkg.add_helper(RegistryHelper::always("canvas_hashScene", HASH_SCENE));
    pkg.add_helper(RegistryHelper::always("canvas_renderScene", RENDER_SCENE));
    pkg.add_helper(RegistryHelper::always("canvas_renderMetal", RENDER_METAL));
}
