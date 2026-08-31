//! The surface: its size, and where a rendered frame goes.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The canvas surface's pixel dimensions.
///
/// Read from the graphics state, which the platform's resize event publishes
/// (plan-98-D Phase 3). Unpublished reads as the startup size — the three platform
/// surfaces are all created 900x640 — so a program that never resizes sees exactly
/// what it saw when this was a constant.
#[rustfmt::skip]
const SURFACE_SIZE: &str =
r#"FUNC __canvas_surfaceSize() AS Size
  RETURN Size[width := canvas::surfaceWidth(), height := canvas::surfaceHeight()]
END FUNC"#;

/// Hand a finished frame to the surface.
///
/// Two destinations, and both are real paths rather than one path and a placeholder:
///
/// * When `MFB_CANVAS_DUMP` names a file, the raw RGBA8 bytes are written to it.
///   That is how a *headless* run — which has no window at all — is observed, and it
///   is what makes the rasteriser testable as an oracle rather than only visible.
///   The same shape as the existing `MFB_WINAPP_DUMP` transcript readback.
/// * The window blit is plan-98-C Phase 3, per platform.
///
/// A frame with no destination is not an error: a program can legitimately render
/// before its surface exists (between `setMode` and the reconcile completing), and
/// the next frame will land.
///
/// `MFB_CANVAS_STATS` is the same idea for the geometry cache. The cache's whole
/// claim is that re-presenting an unchanged item generates nothing, and that claim
/// is invisible from the pixels — an identical frame is what you get whether the
/// geometry was reused or rebuilt. Writing the counters out is what makes plan-98-A
/// invariant 2 testable instead of merely asserted.
///
/// It **appends** one line per rendered frame rather than overwriting. The
/// interesting quantity is the *delta* between frames — "this present generated
/// nothing" — and a file holding only the final total cannot show it.
///
/// It also carries the renderer selection: `metal=` (a Metal device exists),
/// `gpuSelected=` (the program asked for it), `metalReady=` (the Metal pipeline
/// built) and `vulkan=` (a Vulkan device exists — plan-98-F). All are internal-only,
/// so a program cannot call them and a test cannot read them any other way; putting
/// them here makes the seam's discriminants observable without adding public surface
/// for a test to poke.
///
/// `vulkanReady=` is what proves the whole Vulkan path end to end: it is FALSE
/// unless `dlopen("libvulkan.so.1")` succeeded, the entry points resolved, an
/// instance was created from hand-written structs, a physical device with a graphics
/// queue was found, a logical device and queue were made, and the shader modules,
/// layout, render pass and pipeline all built.
///
/// There was briefly a second, narrower `vulkan=` probe reporting only "a device
/// exists". It is gone: it disagreed with this one on box 2227 — reporting FALSE on a
/// machine where the pipeline demonstrably built and rendered a byte-identical frame
/// — and two probes of overlapping facts that can disagree are worse than one, since
/// the narrower one answered a question nothing gated on.
///
/// `metalReady` is what actually builds the device, queue and pipeline the first
/// time it is asked — the same call the renderer branch makes — so a stats line
/// reporting `metalReady=TRUE` is evidence the whole setup ran on the graphics
/// thread, not merely that a device was found.
#[rustfmt::skip]
const PRESENT_SURFACE: &str =
r#"FUNC __canvas_presentSurface(buffer AS List OF Byte, width AS Integer, height AS Integer) AS Nothing
  __canvas_writeStats()
  canvas::blitSurface(buffer, width, height)
  LET path AS String = os::getEnvOr("MFB_CANVAS_DUMP", "")
  IF len(path) > 0 THEN
    fs::writeBytes(path, buffer) TRAP(err)
      RETURN
    END TRAP
  END IF
END FUNC

' The damage rectangle as `x,y,w,h`, or `none`. A test that wants to know a partial
' redraw happened has to be able to see WHICH rectangle -- "partial=1" alone would pass
' just as happily for a rectangle covering the whole window.
FUNC __canvas_damageText() AS String
  IF len(__CANVAS_DAMAGE) < 4 THEN
    RETURN "none"
  END IF
  LET x0 AS Integer = collections::getOr(__CANVAS_DAMAGE, 0, 0)
  LET y0 AS Integer = collections::getOr(__CANVAS_DAMAGE, 1, 0)
  RETURN toString(x0) & "," & toString(y0) & "," & toString(collections::getOr(__CANVAS_DAMAGE, 2, 0) - x0) & "," & toString(collections::getOr(__CANVAS_DAMAGE, 3, 0) - y0)
END FUNC

FUNC __canvas_writeStats() AS Nothing
  LET path AS String = os::getEnvOr("MFB_CANVAS_STATS", "")
  IF len(path) > 0 THEN
    LET line AS String = "generations=" & toString(__CANVAS_GEO_GENERATIONS) & " entries=" & toString(len(__CANVAS_GEO_HASHES)) & " floats=" & toString(len(__CANVAS_GEO_DATA)) & " metal=" & toString(canvas::metalAvailable()) & " gpuSelected=" & toString(canvas::useGpu()) & " metalReady=" & toString(canvas::metalReady()) & " vulkanReady=" & toString(canvas::vulkanReady()) & " glyphs=" & toString(len(__CANVAS_GLYPH_KEYS)) & " glyphBytes=" & toString(len(__CANVAS_GLYPH_COV)) & " glyphEvictions=" & toString(__CANVAS_GLYPH_EVICTIONS) & " frames=" & toString(__CANVAS_FRAMES) & " skipped=" & toString(__CANVAS_SKIPPED) & " partial=" & toString(__CANVAS_PARTIAL) & " damage=" & __canvas_damageText() & "\n"
    fs::appendText(path, line) TRAP(err)
      RETURN
    END TRAP
  END IF
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_surfaceSize", SURFACE_SIZE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_presentSurface",
        PRESENT_SURFACE,
    ));
}
