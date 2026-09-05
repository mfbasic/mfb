//! The canvas colour pipeline: compositing, blend modes, and gradient interpolation.
//!
//! **Every value here is deterministic on purpose.** The software rasteriser is the
//! oracle plan-98-E/F compare their GPU output against, so it must produce the same
//! bytes on every target. That rules out `pow` and every other libm transcendental,
//! whose results differ across platforms.
//!
//! IEEE-754 `+`, `-`, `*`, `/` and `sqrt` ARE exactly specified and so are safe; only
//! transcendentals are not. The rasteriser is written to that line.
//!
//! **The sRGB transfer itself no longer lives here.** plan-122-B moved the 256-entry
//! table and its binary-search inverse into `color`, where they are the public
//! `color::toLinear` / `color::fromLinear` pair (`builtins::color::helper_srgb`), and
//! every function below calls that pair instead of indexing a canvas-local
//! `__CANVAS_SRGB`. The move is value-preserving by construction — same literals,
//! same search, same rounding — and the four unit tests that pin the table's length,
//! endpoints, monotonicity and agreement with the transfer function moved with it.
//!
//! Public rather than a private helper canvas reaches across the boundary, because a
//! package cannot reach another package's private `__` members. Duplicating the table
//! instead would have left two copies that must agree forever, with `color::luminance`
//! silently disagreeing with what canvas renders as the failure mode.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// One channel of the over-operator, in linear space.
///
/// `dst + (src - dst) * alpha`, evaluated on the linear values and converted back.
/// Rounding is round-to-nearest via the `+ 127` before the divide, so
/// `blendChannel(d, s, 255) == s` holds exactly rather than drifting a step low.
#[rustfmt::skip]
const BLEND_CHANNEL: &str =
r#"FUNC __canvas_blendChannel(dst AS Byte, src AS Byte, alpha AS Integer) AS Byte
  LET dstLin AS Integer = color::toLinear(dst)
  LET srcLin AS Integer = color::toLinear(src)
  LET mixed AS Integer = dstLin + ((srcLin - dstLin) * alpha + 127) / 255
  RETURN color::fromLinear(mixed)
END FUNC"#;

/// The same over-operator, with a `BlendMode` applied to the source first.
///
/// **Mode `0` is bit-for-bit `__canvas_blendChannel`.** That is the whole
/// compatibility contract of plan-116-B: `BlendMode.Normal` is the zero value, every
/// `Paint` ever built carries it, and every existing golden renders through it. The
/// `Normal` arm therefore does not merely *compute* the same thing — it is the same
/// expression, so it cannot drift by a rounding step.
///
/// The three others are the standard separable blend functions on **linear** values
/// (`06_canvas.md` §"Rendering conventions" defines them there, not on sRGB bytes),
/// applied to the source and then composited over the destination by the ordinary
/// alpha step. Writing it in that order is what makes a partially-covered pixel right:
/// coverage scales *how much of the blended result lands*, it does not scale the
/// operands.
///
/// The linear table runs `0..65535`, so a product needs the `+ 32767` round-to-nearest
/// and a divide by 65535 — not 65536. Dividing by 65536 (a shift) would make
/// `multiply(x, white)` come out one step below `x`, and white is exactly the
/// destination a `Multiply` item is most often tested against.
#[rustfmt::skip]
const BLEND_CHANNEL_MODE: &str =
r#"FUNC __canvas_blendChannelMode(dst AS Byte, src AS Byte, alpha AS Integer, mode AS Integer) AS Byte
  LET dstLin AS Integer = color::toLinear(dst)
  LET srcLin AS Integer = color::toLinear(src)
  MUT blended AS Integer = srcLin
  IF mode = 1 THEN
    blended = (srcLin * dstLin + 32767) / 65535
  END IF
  IF mode = 2 THEN
    blended = srcLin + dstLin - (srcLin * dstLin + 32767) / 65535
  END IF
  ' `Add` is the one mode that does NOT go through the lerp below, and the reason is
  ' reproducibility rather than taste. Additive blending means "add the COVERED source
  ' to the destination, then clamp" -- coverage scales how much source is added, which
  ' is the premultiplied-source form every GPU expresses as the factor pair (One, One).
  '
  ' Lerping towards a pre-clamped sum instead -- `dst + (min(src+dst,1) - dst)*a` --
  ' agrees at full coverage and diverges by up to 0.15 in linear at partial coverage
  ' over a bright destination, which no fixed-function blend can reproduce. That would
  ' have made `Add` the one mode the GPU backends had to decline (plan-116-B C6).
  IF mode = 3 THEN
    LET added AS Integer = dstLin + (srcLin * alpha + 127) / 255
    IF added > 65535 THEN
      RETURN color::fromLinear(65535)
    END IF
    RETURN color::fromLinear(added)
  END IF
  LET mixed AS Integer = dstLin + ((blended - dstLin) * alpha + 127) / 255
  RETURN color::fromLinear(mixed)
END FUNC"#;

/// The colour a gradient shows at `t`, interpolated **in linear light** (plan-116-F).
///
/// The stop walk is linear rather than a binary search: the stop count is small and
/// bounded, and a linear walk is the shape `__canvas_edgeDistance` already uses per
/// pixel for a polygon's edges.
///
/// **Linear light, not sRGB space**, and that is a decision rather than an oversight.
/// It is the space `06_canvas.md` §"Rendering conventions" already composites in, and
/// the space both shaders' `srgbToLinear` puts colours into, so a gradient blends the
/// way everything else on the surface does. It is also the choice that makes a
/// black-to-white ramp look uniformly bright: interpolating the *encoded* bytes spends
/// half the ramp below 22% of the light, which reads as dark-heavy. `gradients.png`'s
/// fourth row is that ramp, so the choice is inspectable rather than asserted.
///
/// Offsets arrive already clamped to `0..1` and monotonic (`__canvas_gradientTail`),
/// so the walk needs no defence against a stop that goes backwards. Before the first
/// stop and after the last, the end stop's colour holds rather than extrapolating.
#[rustfmt::skip]
const GRADIENT_COLOR: &str =
r#"FUNC __canvas_gradientChannel(loSrgb AS Integer, hiSrgb AS Integer, num AS Integer, den AS Integer) AS Byte
  LET loLin AS Integer = color::toLinear(toByte(loSrgb))
  LET hiLin AS Integer = color::toLinear(toByte(hiSrgb))
  IF den <= 0 THEN
    RETURN color::fromLinear(loLin)
  END IF
  RETURN color::fromLinear(loLin + (hiLin - loLin) * num / den)
END FUNC

FUNC __canvas_gradientStopColor(at AS Integer) AS Color
  RETURN Color[red := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 1, 0.0))), green := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 2, 0.0))), blue := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 3, 0.0))), alpha := toByte(toInt(collections::getOr(__CANVAS_GEO_DATA, at + 4, 0.0)))]
END FUNC

FUNC __canvas_gradientColor(base AS Integer, count AS Integer, t AS Float) AS Color
  IF count < 2 THEN
    RETURN __canvas_transparent()
  END IF
  MUT tt AS Float = t
  IF tt < 0.0 THEN
    tt = 0.0
  END IF
  IF tt > 1.0 THEN
    tt = 1.0
  END IF
  ' The first stop at or past `tt`. Written as a full walk with a separate index
  ' rather than an early exit, because the obvious early-exit form clobbers the very
  ' index it found: setting the loop counter past `count` to leave the loop loses it,
  ' and the lerp below then reads five slots past the stops into the NEXT record's
  ' header -- which renders as a plausible flat colour rather than as a failure.
  MUT idx AS Integer = count
  MUT i AS Integer = 0
  WHILE i < count
    IF idx = count THEN
      IF collections::getOr(__CANVAS_GEO_DATA, base + i * 5, 0.0) >= tt THEN
        idx = i
      END IF
    END IF
    i = i + 1
  END WHILE
  IF idx >= count THEN
    RETURN __canvas_gradientStopColor(base + (count - 1) * 5)
  END IF
  IF idx <= 0 THEN
    RETURN __canvas_gradientStopColor(base)
  END IF
  LET loAt AS Integer = base + (idx - 1) * 5
  LET hiAt AS Integer = base + idx * 5
  LET loOff AS Float = collections::getOr(__CANVAS_GEO_DATA, loAt, 0.0)
  LET hiOff AS Float = collections::getOr(__CANVAS_GEO_DATA, hiAt, 0.0)
  MUT num AS Integer = 0
  LET span AS Float = hiOff - loOff
  IF span > 0.0 THEN
    num = toInt((tt - loOff) / span * 4096.0)
  END IF
  IF num < 0 THEN
    num = 0
  END IF
  IF num > 4096 THEN
    num = 4096
  END IF
  LET loA AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 4, 0.0))
  LET hiA AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 4, 0.0))
  RETURN Color[red := __canvas_gradientChannel(toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 1, 0.0)), toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 1, 0.0)), num, 4096), green := __canvas_gradientChannel(toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 2, 0.0)), toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 2, 0.0)), num, 4096), blue := __canvas_gradientChannel(toInt(collections::getOr(__CANVAS_GEO_DATA, loAt + 3, 0.0)), toInt(collections::getOr(__CANVAS_GEO_DATA, hiAt + 3, 0.0)), num, 4096), alpha := toByte(loA + (hiA - loA) * num / 4096)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_blendChannel", BLEND_CHANNEL));
    pkg.add_helper(RegistryHelper::always(
        "canvas_gradientColor",
        GRADIENT_COLOR,
    ));
    pkg.add_helper(RegistryHelper::always(
        "canvas_blendChannelMode",
        BLEND_CHANNEL_MODE,
    ));
}
