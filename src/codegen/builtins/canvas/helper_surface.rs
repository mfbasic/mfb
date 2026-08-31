//! The surface: its size, and where a rendered frame goes.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The canvas surface's pixel dimensions.
///
/// The three platform surfaces plan-98-A builds are all created at 900x640
/// (`emit_reconcile_canvas_helper`, `RECONCILE_BUILD_SYMBOL`, the Win32
/// `CreateWindowExW`), so that is genuinely the size — not a placeholder. Live
/// resize is plan-98-D's, which is also where this stops being a constant and
/// becomes a query against the presented surface.
#[rustfmt::skip]
const SURFACE_SIZE: &str =
r#"FUNC __canvas_surfaceSize() AS Size
  RETURN Size[width := 900, height := 640]
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
#[rustfmt::skip]
const PRESENT_SURFACE: &str =
r#"FUNC __canvas_presentSurface(buffer AS List OF Byte, width AS Integer, height AS Integer) AS Nothing
  __canvas_writeStats()
  LET path AS String = os::getEnvOr("MFB_CANVAS_DUMP", "")
  IF len(path) > 0 THEN
    fs::writeBytes(path, buffer) TRAP(err)
      RETURN
    END TRAP
  END IF
END FUNC

FUNC __canvas_writeStats() AS Nothing
  LET path AS String = os::getEnvOr("MFB_CANVAS_STATS", "")
  IF len(path) > 0 THEN
    LET line AS String = "generations=" & toString(__CANVAS_GEO_GENERATIONS) & " entries=" & toString(len(__CANVAS_GEO_HASHES)) & " floats=" & toString(len(__CANVAS_GEO_DATA)) & "\n"
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
