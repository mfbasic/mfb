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
    canvas::startGraphics()
    __CANVAS_GFX_READY = TRUE
  END IF
END FUNC"#;

/// The graphics thread's whole life: wait, render, repeat.
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
    __canvas_renderScene()
    canvas::frameDone()
  END WHILE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_ensureGraphics",
        ENSURE_GRAPHICS,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_renderLoop", RENDER_LOOP));
    pkg.add_helper(RegistryHelper::always("canvas_hashScene", HASH_SCENE));
    pkg.add_helper(RegistryHelper::always("canvas_renderScene", RENDER_SCENE));
}
