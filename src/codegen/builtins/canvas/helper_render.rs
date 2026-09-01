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
r#"FUNC __canvas_renderScene(offsets AS List OF Integer, damage AS List OF Integer, width AS Integer, height AS Integer) AS Nothing
  LET full AS Boolean = __canvas_damageIsFull(damage, width, height)
  MUT buffer AS List OF Byte = []
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
  FOR EACH offset IN offsets
    IF full OR __canvas_boundsMeet(offset, damage) THEN
      buffer = __canvas_drawGeometry(buffer, width, height, offset)
    END IF
  NEXT
  __CANVAS_KEPT = buffer
  __CANVAS_KEPT_W = width
  __CANVAS_KEPT_H = height
  __canvas_presentSurface(buffer, width, height)
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
/// **A glyph run is bounded rather than refused**, and the two bounds have different
/// shapes for a real reason. Metal's bitmap rides `setFragmentBytes:`, copied into the
/// command buffer per draw, so its cap is 4 KiB **per glyph** — about 64x64, a glyph at
/// roughly 200 px. Vulkan's bitmaps are copied into one buffer that serves the whole
/// recording, so its cap is a **frame** total. Neither truncates: a clipped glyph is a
/// different glyph and would read as a rasteriser bug.
///
/// Both predicates declined `__CANVAS_GEO_TEXT` outright for as long as neither backend
/// could draw one, and that was not caution — the version before it *accepted* a kind
/// neither shader knew, and Metal returned a frame with the text simply missing and no
/// error anywhere. 4,536 pixels wrong, reported as success. That is the lie these
/// predicates exist to prevent, and it is why the bound is checked here rather than
/// discovered in the emitter.
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
  ' Published as it goes, not at the end. The geometry cache is smaller than a large
  ' scene, so resolving item 300 can evict item 1 -- while this frame is still holding
  ' item 1's offset and has not drawn it yet. `__canvas_glyphEvict` reads this list to
  ' know that those glyphs are live; without it a 300-item scene lost six of them,
  ' silently, because their cache indices were renumbered out from under the offsets
  ' this function had already returned.
  __CANVAS_GEO_LIVE = []
  FOR EACH item IN canvas::installedItems()
    ' The result lands in a local first: `__canvas_geometryFor` can run an eviction pass
    ' that reassigns `__CANVAS_GEO_LIVE`, and appending to a global whose operand was
    ' resolved before the call writes into the block that pass released
    ' (`.ai/collections.md`).
    LET offset AS Integer = __canvas_geometryFor(item, collections::getOr(hashes, index, 0))
    offsets = collections::append(offsets, offset)
    __CANVAS_GEO_LIVE = collections::append(__CANVAS_GEO_LIVE, offset)
    index = index + 1
  NEXT
  FOR EACH layer IN canvas::installedLayers()
    FOR EACH item IN layer.items
      LET offset AS Integer = __canvas_geometryFor(item, collections::getOr(hashes, index, 0))
      offsets = collections::append(offsets, offset)
      __CANVAS_GEO_LIVE = collections::append(__CANVAS_GEO_LIVE, offset)
      index = index + 1
    NEXT
  NEXT
  RETURN offsets
END FUNC

' The frame's item buffer holds one block per drawn QUAD, and both backends index it by
' instance -- so this is the one cap that is neither Metal's nor Vulkan's, it is the
' shared transport's (plan-116-A, `CANVAS_MAX_FRAME_ITEMS`). A shape is one quad; a glyph
' run is one per glyph, because each glyph is its own quad with its own block.
'
' Counting the run's whole glyph count over-estimates by the glyphs whose cache entry the
' eviction pass dropped -- those draw nothing and take no block. Over-estimating declines
' a hair early, which is the safe direction: under-estimating would let the emitter write
' past the mapping.
LET __CANVAS_MAX_FRAME_ITEMS AS Integer = 4096

LET __CANVAS_METAL_MAX_EDGES AS Integer = 256

LET __CANVAS_METAL_MAX_GLYPH_SAMPLES AS Integer = 4096

' The largest bitmap in a glyph run. Metal's cap is PER GLYPH, not per frame, because
' its bitmaps ride `setFragmentBytes:` -- the same payload its edges ride -- and that is
' copied into the command buffer per draw. Vulkan's is per frame for the opposite
' reason: one buffer serves the whole recording.
FUNC __canvas_runLargestGlyph(offset AS Integer) AS Integer
  LET glyphs AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
  MUT worst AS Integer = 0
  MUT g AS Integer = 0
  WHILE g < glyphs
    LET entry AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + g * 3, 0.0))
    IF entry >= 0 THEN
      LET base AS Integer = entry * 5
      LET samples AS Integer = collections::getOr(__CANVAS_GLYPH_META, base + 2, 0) * collections::getOr(__CANVAS_GLYPH_META, base + 3, 0)
      IF samples > worst THEN
        worst = samples
      END IF
    END IF
    g = g + 1
  END WHILE
  RETURN worst
END FUNC

FUNC __canvas_metalRenderable(offsets AS List OF Integer) AS Boolean
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      IF __canvas_runLargestGlyph(offset) > __CANVAS_METAL_MAX_GLYPH_SAMPLES THEN
        RETURN FALSE
      END IF
    END IF
    IF kind = __CANVAS_GEO_POLYGON THEN
      IF toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0)) > __CANVAS_METAL_MAX_EDGES THEN
        RETURN FALSE
      END IF
    END IF
  NEXT
  RETURN TRUE
END FUNC

FUNC __canvas_renderMetal(offsets AS List OF Integer, width AS Integer, height AS Integer) AS Boolean
  IF NOT __canvas_metalRenderable(offsets) THEN
    RETURN FALSE
  END IF
  LET buffer AS List OF Byte = canvas::newSurface(width, height)
  canvas::metalDrawScene(buffer, width, height, __CANVAS_GEO_DATA, offsets, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV)
  __CANVAS_KEPT = buffer
  __CANVAS_KEPT_W = width
  __CANVAS_KEPT_H = height
  __canvas_presentSurface(buffer, width, height)
  RETURN TRUE
END FUNC

LET __CANVAS_VULKAN_MAX_FRAME_EDGES AS Integer = 16384
LET __CANVAS_VULKAN_MAX_GLYPH_SAMPLES AS Integer = 1048576

' The coverage samples one glyph run puts in the frame's glyph region -- the sum of its
' cached bitmaps' areas. A run carries cache indices rather than bitmaps, so this is the
' only place the two caches meet, and it is why the predicate takes the shape it does:
' the question "does this frame fit" cannot be answered from the geometry alone.
FUNC __canvas_runSamples(offset AS Integer) AS Integer
  LET glyphs AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
  MUT total AS Integer = 0
  MUT g AS Integer = 0
  WHILE g < glyphs
    LET entry AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + g * 3, 0.0))
    IF entry >= 0 THEN
      LET base AS Integer = entry * 5
      total = total + collections::getOr(__CANVAS_GLYPH_META, base + 2, 0) * collections::getOr(__CANVAS_GLYPH_META, base + 3, 0)
    END IF
    g = g + 1
  END WHILE
  RETURN total
END FUNC

FUNC __canvas_vulkanRenderable(offsets AS List OF Integer) AS Boolean
  MUT total AS Integer = 0
  MUT samples AS Integer = 0
  MUT quads AS Integer = 0
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      samples = samples + __canvas_runSamples(offset)
      quads = quads + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    ELSE
      quads = quads + 1
    END IF
    IF kind = __CANVAS_GEO_POLYGON THEN
      total = total + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    END IF
  NEXT
  IF quads > __CANVAS_MAX_FRAME_ITEMS THEN
    RETURN FALSE
  END IF
  IF samples > __CANVAS_VULKAN_MAX_GLYPH_SAMPLES THEN
    RETURN FALSE
  END IF
  RETURN total <= __CANVAS_VULKAN_MAX_FRAME_EDGES
END FUNC

FUNC __canvas_renderVulkan(offsets AS List OF Integer, width AS Integer, height AS Integer) AS Boolean
  IF NOT __canvas_vulkanRenderable(offsets) THEN
    RETURN FALSE
  END IF
  LET buffer AS List OF Byte = canvas::newSurface(width, height)
  canvas::vulkanDrawScene(buffer, width, height, __CANVAS_GEO_DATA, offsets, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV)
  __CANVAS_KEPT = buffer
  __CANVAS_KEPT_W = width
  __CANVAS_KEPT_H = height
  __canvas_presentSurface(buffer, width, height)
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
  LET size AS Size = __canvas_surfaceSize()
  ' The geometry is built once, here, and the offsets are then handed to whichever
  ' backend draws them. It has to happen before the damage diff rather than inside the
  ' renderer: an item's damaged rectangle is its geometry's bounds, so there is no
  ' diff to compute until the geometry exists.
  LET offsets AS List OF Integer = __canvas_sceneOffsets()
  LET hashes AS List OF Integer = canvas::installedHashes()
  LET damage AS List OF Integer = __canvas_damageFor(hashes, offsets, size.width, size.height)
  __CANVAS_DAMAGE = damage
  IF len(damage) = 0 THEN
    ' Nothing changed, so nothing is presented. The loop still calls `frameDone`, so a
    ' `present` waiting under MFB_CANVAS_SYNC is released -- a skipped frame is a frame
    ' that finished, not one that was lost.
    __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
    __canvas_writeStats()
    RETURN
  END IF
  __CANVAS_FRAMES = __CANVAS_FRAMES + 1
  IF NOT __canvas_damageIsFull(damage, size.width, size.height) THEN
    __CANVAS_PARTIAL = __CANVAS_PARTIAL + 1
  END IF
  __canvas_rememberScene(hashes, offsets)

  ' A GPU backend renders the whole frame whatever the damage is: it draws into its own
  ' texture and reads the result back, so there is no kept surface for it to preserve.
  ' That is the plan's "capability-absent path falls back to full-frame", and it is the
  ' honest reading here -- the capability the plan named (`VK_KHR_incremental_present`,
  ' Metal dirty rects) belongs to presenting a swapchain drawable, which this renderer
  ' does not do at all (Correction 24).
  IF canvas::useGpu() AND canvas::metalReady() THEN
    IF __canvas_renderMetal(offsets, size.width, size.height) THEN
      RETURN
    END IF
  END IF
  IF canvas::useGpu() AND canvas::vulkanReady() THEN
    IF __canvas_renderVulkan(offsets, size.width, size.height) THEN
      RETURN
    END IF
  END IF
  __canvas_renderScene(offsets, damage, size.width, size.height)
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
        GEO_KIND_POLYGON, GEO_KIND_TEXT, HEADER_AUX0, MAX_EDGES, METAL_MAX_GLYPH_SAMPLES,
        VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES,
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
        assert_eq!(
            declared("__CANVAS_VULKAN_MAX_GLYPH_SAMPLES"),
            VULKAN_MAX_FRAME_GLYPH_SAMPLES,
            "the predicate admits a frame whose glyph bitmaps the buffer's glyph \
             region cannot hold",
        );
        assert_eq!(
            declared("__CANVAS_METAL_MAX_GLYPH_SAMPLES"),
            METAL_MAX_GLYPH_SAMPLES,
            "the predicate admits a glyph bigger than `setFragmentBytes:` will carry",
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
            4,
            "the two edge sums and both glyph-run walks should all read HEADER_AUX0"
        );
    }

    /// A glyph run is a kind the predicates now *admit* rather than refuse, so its
    /// spelling has to be as pinned as the polygon's — and by the same argument. It
    /// was not always: both predicates declined `__CANVAS_GEO_TEXT` outright until
    /// the backends could draw one, and the version before *that* accepted it while
    /// neither shader knew the kind, returning a frame with the text missing and
    /// calling it success.
    #[test]
    fn the_text_kind_is_spelled_once() {
        assert_eq!(GEO_KIND_TEXT, "6");
        assert!(RENDER_METAL.contains("= __CANVAS_GEO_TEXT THEN"));
    }

    /// Both predicates test the same kind the emitters and the shaders branch on.
    #[test]
    fn the_polygon_kind_is_spelled_once() {
        assert_eq!(GEO_KIND_POLYGON, "4");
        assert!(RENDER_METAL.contains("= __CANVAS_GEO_POLYGON THEN"));
    }
}
