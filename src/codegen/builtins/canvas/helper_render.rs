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
/// `__canvas_metalRenderable` is the honesty gate: the renderer draws a scene only
/// when every item is one it reproduces, and hands the rest back to the oracle rather
/// than drawing it wrongly.
///
/// Phase 2's SDF fragment shader evaluates the same distance functions the software
/// rasteriser does, so every kind now passes — including `__CANVAS_GEO_NONE`, which
/// both backends draw as nothing. **One condition remains**: a polygon's edges cross
/// as a `setFragmentBytes:` payload, which Metal caps at 4 KB, so a polygon past
/// `__CANVAS_METAL_MAX_EDGES` is declined. Clamping it instead would render a
/// *different polygon* and read as a geometry bug.
///
/// It reads the geometry header by slot rather than through a helper because that is
/// what the header is for: a fixed 22-float layout both backends index directly
/// (`__canvas_headerFor`).
///
/// `__canvas_vulkanRenderable` is the Vulkan predicate, and it declines a *different*
/// set — which is why it is a second function rather than a share of the first.
/// Vulkan's edges live in a descriptor-bound storage buffer, so there is no per-item
/// limit at all; the limit is the buffer, and the buffer serves the whole frame,
/// because a command buffer is recorded once and executed once. So Metal caps each
/// polygon and Vulkan caps their sum. The two really are different conditions, and a
/// scene can be GPU-renderable on one backend and not the other.
///
/// **Both decline a glyph run** (`__CANVAS_GEO_TEXT`). Neither shader has a sampler, and
/// a glyph run's tail is cache indices rather than edges, so a backend that accepted it
/// would draw nothing while reporting success — the exact lie both predicates exist to
/// prevent, and one this letter watched happen: the first version of the glyph cache
/// left the predicates alone and the GPU frame came back with the text missing and no
/// error anywhere. Restoring GPU text needs a glyph atlas and a sampler, which is
/// plan-98-G Phase 2's own row.
///
/// Sharing the Metal predicate was the tempting shortcut when this shader still
/// declined every polygon, and it would have rendered them as nothing while reporting
/// success — the same lie both predicates exist to prevent. It was measured, not
/// assumed: the scene that found it differed from the oracle on 4,610 pixels, all of
/// them the one triangle.
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

LET __CANVAS_METAL_MAX_EDGES AS Integer = 256

FUNC __canvas_metalRenderable(offsets AS List OF Integer) AS Boolean
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      RETURN FALSE
    END IF
    IF kind = __CANVAS_GEO_POLYGON THEN
      IF toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0)) > __CANVAS_METAL_MAX_EDGES THEN
        RETURN FALSE
      END IF
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
END FUNC

LET __CANVAS_VULKAN_MAX_FRAME_EDGES AS Integer = 16384

FUNC __canvas_vulkanRenderable(offsets AS List OF Integer) AS Boolean
  MUT total AS Integer = 0
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      RETURN FALSE
    END IF
    IF kind = __CANVAS_GEO_POLYGON THEN
      total = total + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    END IF
  NEXT
  RETURN total <= __CANVAS_VULKAN_MAX_FRAME_EDGES
END FUNC

FUNC __canvas_renderVulkan() AS Boolean
  LET offsets AS List OF Integer = __canvas_sceneOffsets()
  IF NOT __canvas_vulkanRenderable(offsets) THEN
    RETURN FALSE
  END IF
  LET size AS Size = __canvas_surfaceSize()
  LET buffer AS List OF Byte = canvas::newSurface(size.width, size.height)
  canvas::vulkanDrawScene(buffer, size.width, size.height, __CANVAS_GEO_DATA, offsets)
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
    canvas::setGpuMode(len(os::getEnvOr("MFB_CANVAS_GPU", "")) > 0)
    canvas::startGraphics()
    __CANVAS_GFX_READY = TRUE
  END IF
END FUNC"#;

/// The graphics thread's whole life: wait, render, repeat — and the renderer seam.
///
/// `__canvas_renderFrame` is the one place that will choose a renderer (plan-98-E).
/// It is deliberately a runtime branch rather than a build-time one, because the
/// choice is a runtime fact: whether a Metal device exists, and whether the program
/// asked for it (`canvas::metalAvailable` and `canvas::useGpu` answer those).
///
/// There are two GPU arms — Metal on macOS, Vulkan on Linux — and each is taken only
/// when all three of its conditions hold: the program asked for a GPU
/// (`canvas::useGpu`, which despite its name is the one renderer-selection flag and
/// is set by `MFB_CANVAS_GPU`), a pipeline exists (`canvas::metalReady` /
/// `canvas::vulkanReady`), and the *scene* is one that renderer draws correctly.
///
/// The two are mutually exclusive in practice — `metalReady` is FALSE off macOS and
/// `vulkanReady` FALSE off Linux — so the order between them never decides anything;
/// they are written as separate `IF`s rather than an `ELSE` so a future platform with
/// both is a matter of which is listed first, not a restructure.
///
/// That third condition is what keeps `MFB_CANVAS_GPU=1` honest: a backend that
/// drew a circle as its bounding box would still *report* success, which is exactly
/// the lie Correction 3 rejected. Since Phase 2 the shader reproduces every
/// primitive, so the only scene still declined is one carrying a polygon with more
/// edges than a `setFragmentBytes:` payload holds.
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
  IF canvas::useGpu() AND canvas::metalReady() THEN
    IF __canvas_renderMetal() THEN
      RETURN
    END IF
  END IF
  IF canvas::useGpu() AND canvas::vulkanReady() THEN
    IF __canvas_renderVulkan() THEN
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::runtime::canvas::{
        GEO_KIND_POLYGON, HEADER_AUX0, MAX_EDGES, VULKAN_MAX_FRAME_EDGES,
    };

    /// Find `LET <name> AS Integer = <n>` in the injected MFBASIC source.
    fn declared(name: &str) -> usize {
        let needle = format!("LET {name} AS Integer = ");
        let start = RENDER_METAL
            .find(&needle)
            .unwrap_or_else(|| panic!("{name} is not declared in RENDER_METAL"))
            + needle.len();
        let rest = &RENDER_METAL[start..];
        let end = rest.find('\n').unwrap_or(rest.len());
        rest[..end].trim().parse().expect("a decimal literal")
    }

    /// The predicates are written in MFBASIC and the emitters in Rust, so the limits
    /// exist twice — with no compiler between them. Drift is not a style problem: if
    /// the MFBASIC cap ever exceeds the Rust one, the predicate admits a scene the
    /// emitter's buffer cannot hold, and Metal's is a *stack* buffer.
    #[test]
    fn the_two_gpu_edge_budgets_match_the_emitters() {
        assert_eq!(declared("__CANVAS_METAL_MAX_EDGES"), MAX_EDGES);
        assert_eq!(
            declared("__CANVAS_VULKAN_MAX_FRAME_EDGES"),
            VULKAN_MAX_FRAME_EDGES
        );
    }

    /// The predicates read the geometry header by slot. `offset + 20` is
    /// `HEADER_AUX0` — the polygon's edge count — and a renumbered header would leave
    /// them summing an arc's start angle instead, which is a plausible-looking number
    /// rather than an error.
    #[test]
    fn the_predicates_read_the_edge_count_slot() {
        assert_eq!(HEADER_AUX0, 20);
        assert_eq!(
            RENDER_METAL
                .matches(&format!("offset + {HEADER_AUX0}"))
                .count(),
            2,
            "both predicates should read the edge count from HEADER_AUX0"
        );
    }

    /// Both predicates test the same kind the emitters and the shaders branch on.
    #[test]
    fn the_polygon_kind_is_spelled_once() {
        assert_eq!(GEO_KIND_POLYGON, "4");
        assert!(RENDER_METAL.contains("= __CANVAS_GEO_POLYGON THEN"));
    }
}
