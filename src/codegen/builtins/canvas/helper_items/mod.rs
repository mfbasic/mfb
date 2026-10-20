//! `__canvas_drawGeometry` — the one rasterisation loop.
//!
//! The rasteriser never sees a `DrawItem`. It reads the flat float record
//! `helper_geometry.rs` produced, which is the same buffer plan-98-E/F upload to the
//! GPU — so the oracle and the GPU backends consume one representation rather than
//! two that could drift.
//!
//! ## Why one loop, and why the pixel writes are inline
//!
//! Every `collections::set` on the surface **must** appear in the function that owns
//! the surface local, spelled `out = collections::set(out, i, v)`. That exact shape is
//! what `try_inplace_set_assign` recognises; anything else falls back to copying the
//! whole 2.3 MB list per write.
//!
//! This is not a micro-optimisation. Measured on 20,000 writes into a 200,000-byte
//! list:
//!
//! | shape | time |
//! |---|---|
//! | `out = collections::set(out, i, v)` on a local | **5 ms** |
//! | threaded through a helper's parameter and return | 1179 ms |
//! | through a module-level `MUT` | 1161 ms |
//!
//! 290x. The first version of this rasteriser had a `__canvas_blendPixel(surface, …)
//! AS List OF Byte` helper, and one 900x640 frame took **53 seconds**. A read
//! (`collections::getOr`) is free either way, so only the writes are constrained —
//! which is why the blend *arithmetic* still lives in `helper_color.rs` and only the
//! four `set` calls are inline here.
//!
//! Collapsing the four separate loops (rect/circle span, stroke span, arc, polygon)
//! into this one is what keeps that inlining from becoming four copies of the same
//! blend block. The loops only ever differed by their distance function, which is now
//! `__canvas_geoDistance`.
//!
//! ## Fill then stroke
//!
//! In that order, matching every 2D API a user is likely to have met: an outline drawn
//! under its own fill would be half-hidden by it. `Line` and `Arc` have no interior,
//! so their generator puts the *stroke* colour in the fill slots and leaves the stroke
//! half-width negative — one pass, no special case here.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

mod draw_geometry;
mod geo_read;
use draw_geometry::DRAW_GEOMETRY;
use geo_read::GEO_READ;

/// The half-width of a stroke, or a negative number when there is nothing to stroke.
///
/// Folding "is there a stroke at all" into the same value the band test uses keeps the
/// caller from repeating the two-part check (`alpha > 0` and `width > 0`) everywhere.
#[rustfmt::skip]
const STROKE_HALF: &str =
r#"FUNC __canvas_strokeHalf(paint AS Paint) AS Float
  ' plan-163-D: a stroke the gradient paints (`FillAndStroke`) is the ramp tinted by the
  ' stroke colour, whose alpha is only the tint's strength -- so a zero-alpha stroke
  ' still strokes, in the plain ramp.
  LET ramped AS Boolean = len(paint.gradient.stops) >= 2 AND paint.gradient.strokeApply = GradientApply.FillAndStroke
  IF toInt(paint.stroke.alpha) <= 0 AND NOT ramped THEN
    RETURN 0.0 - 1.0
  END IF
  IF paint.strokeWidth <= 0.0 THEN
    RETURN 0.0 - 1.0
  END IF
  RETURN paint.strokeWidth / 2.0
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_strokeHalf", STROKE_HALF));
    pkg.add_helper(RegistryHelper::always("canvas_geoRead", GEO_READ));
    pkg.add_helper(RegistryHelper::always("canvas_drawGeometry", DRAW_GEOMETRY));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `__canvas_strokeHalf` reports "this paint does not stroke" as a **negative**
    /// value, not zero. Every native consumer therefore has to read it SIGNED.
    ///
    /// plan-116-H: the Vulkan emitter's blend-split test read it with `load_u32`, which
    /// zero-extends, so `-1.0` in 16.16 (`0xFFFF0000`) compared as 4294901760 and every
    /// blended fill-only item was published as TWO records instead of one. Nothing
    /// caught it for as long as the draw call's instance count came from the emitter's
    /// own cursor -- the extra record was drawn and painted nothing. It only became a
    /// visible defect when the draw list started predicting instance counts
    /// independently, at which point one extra record shifted every later draw base and
    /// the scene lost its tail.
    ///
    /// This pins the sentinel because the cheap "fix" from the other direction -- making
    /// it return `0.0` -- would silently paper over a signed-read bug and leave the next
    /// consumer to rediscover this.
    #[test]
    fn a_paint_that_does_not_stroke_reports_a_negative_stroke_half() {
        let negatives = STROKE_HALF.matches("RETURN 0.0 - 1.0").count();
        assert_eq!(
            negatives, 2,
            "__canvas_strokeHalf must return a NEGATIVE sentinel for both \
             non-stroking cases (zero alpha, non-positive width); native consumers \
             read this field signed and a 0.0 sentinel changes what they publish",
        );
        assert!(
            !STROKE_HALF.contains("RETURN 0.0\n"),
            "a bare 0.0 sentinel makes the no-stroke case indistinguishable from a \
             zero-width stroke under an unsigned read",
        );
    }
}
