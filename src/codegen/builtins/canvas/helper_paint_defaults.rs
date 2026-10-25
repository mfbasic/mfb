//! The zero values the `Paint` constructors fill in for the fields a caller did
//! not name.
//!
//! These exist because named construction used to require every field
//! (`Paint[fill := c]` was a `TYPE_CONSTRUCTOR_ARITY_MISMATCH`). A by-field
//! constructor now defaults the fields it omits (bug-1021), and those defaults
//! equal these zeros; `canvas::fill`/`stroke`/`fillStroke` keep writing them
//! explicitly so the "every field's zero value is that field's no-op" rule reads
//! at the call rather than resting on the type's default.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Fully transparent — the no-op `color::Color`, and what an unnamed `Paint` channel is.
#[rustfmt::skip]
const TRANSPARENT: &str =
r#"FUNC __canvas_transparent() AS color::Color
  RETURN color::Color[red := toByte(0), green := toByte(0), blue := toByte(0), alpha := toByte(0)]
END FUNC"#;

/// The all-zero `Transform`, which `canvas` defines to mean the identity — see the
/// `Transform` type description. Writing the literal identity matrix here instead
/// would be wrong: it would make an explicitly-zero transform and a defaulted one
/// behave differently under a later `WITH`.
#[rustfmt::skip]
const NO_TRANSFORM: &str =
r#"FUNC __canvas_noTransform() AS Transform
  RETURN Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0]
END FUNC"#;

/// A `Float` as the bit pattern of the nearest IEEE-754 **binary32**, as a whole
/// number in `0 .. 2^32`.
///
/// **Why this exists in MFBASIC.** Both GPUs read every position as float32. The
/// emitters narrow the geometry header's doubles themselves (`emit_f32_bits`, an `fcvt`),
/// but some float32 values are made before any emitter runs and travel in MFBASIC
/// lists: the inverse transforms in the header (slots 27–33 and the gradient's) and the
/// group offsets and matrices in the draw list (`__canvas_pushOneDraw`). Those are
/// narrowed here, in one implementation an ordinary test can check against known bit
/// patterns.
///
/// Arithmetic, not bit-twiddling: `bits::` takes `Integer` and never `Float`, so there
/// is no reinterpret to borrow. The three fields are assembled by **addition** rather
/// than OR, which is exact because they do not overlap.
///
/// Deliberately simple at the extremes, and the limits are stated rather than
/// discovered: a value too large for binary32 saturates to the largest finite float and
/// one too small flushes to zero, instead of producing an infinity or a denormal. A
/// transform that reaches either is already degenerate — `__canvas_invertTransform`
/// rejects a singular matrix before this is ever called — and an infinity reaching a
/// distance field poisons a whole frame rather than one item.
#[rustfmt::skip]
const FLOAT32_BITS: &str =
r#"FUNC __canvas_float32Bits(value AS Float) AS Integer
  IF value = 0.0 THEN
    RETURN 0
  END IF
  MUT sign AS Integer = 0
  MUT v AS Float = value
  IF v < 0.0 THEN
    sign = 1
    v = 0.0 - v
  END IF
  MUT e AS Integer = 0
  WHILE v >= 2.0 AND e < 128
    v = v / 2.0
    e = e + 1
  END WHILE
  WHILE v < 1.0 AND e > 0 - 127
    v = v * 2.0
    e = e - 1
  END WHILE
  ' Out of binary32's normal range: saturate rather than emit an infinity or a
  ' denormal, so a degenerate transform cannot poison a distance field.
  IF e > 127 THEN
    RETURN sign * 2147483648 + 2139095039
  END IF
  IF e < 0 - 126 THEN
    RETURN sign * 2147483648
  END IF
  MUT mantissa AS Integer = toInt((v - 1.0) * 8388608.0 + 0.5)
  IF mantissa >= 8388608 THEN
    ' The round carried into the exponent: 1.111...1 rounded up is 10.0.
    mantissa = 0
    e = e + 1
    IF e > 127 THEN
      RETURN sign * 2147483648 + 2139095039
    END IF
  END IF
  RETURN sign * 2147483648 + (e + 127) * 8388608 + mantissa
END FUNC"#;

/// Invert a `Transform` once, on the CPU, into the six terms every renderer reads.
///
/// Returns `[ia, ib, ic, id, itx, ity, hasTransform]` — seven floats, laid out to be
/// copied straight into header slots 27–33.
///
/// **This is the only place the all-zero-means-identity rule lives.** `canvas` defines
/// the all-zero `Transform` as the identity rather than the degenerate
/// collapse-to-origin matrix (see the `Transform` type description), and three
/// renderers each re-deriving that would be three chances to disagree with it.
///
/// **A singular transform renders untransformed, not invisible.** `|det| < 1e-12` — a
/// collapse to a line or a point — returns the identity with the flag clear. Drawing
/// nothing would be indistinguishable from never presenting the item; an obviously
/// untransformed item is a visible bug. It also means no renderer can be handed an
/// infinity or a NaN from a transform, which would otherwise reach a distance field
/// and poison a whole frame rather than one item.
///
/// The `hasTransform` flag is computed here too, so "is this the identity" is decided
/// once by the same code that decides what the identity *is*.
#[rustfmt::skip]
const INVERT_TRANSFORM: &str =
r#"FUNC __canvas_invertTransform(t AS Transform) AS List OF Float
  ' The identity, already as float32 bit patterns: 1.0 is 0x3F800000 = 1065353216 and
  ' 0.0 is 0. The trailing 0.0 is the hasTransform flag, which is a plain 0/1 and not a
  ' bit pattern -- it is compared, never decoded.
  LET identity AS List OF Float = [1065353216.0, 0.0, 0.0, 1065353216.0, 0.0, 0.0, 0.0]
  IF t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0 THEN
    RETURN identity
  END IF
  LET det AS Float = t.a * t.d - t.b * t.c
  IF __canvas_absF(det) < 0.000000000001 THEN
    RETURN identity
  END IF
  LET ia AS Float = t.d / det
  LET ib AS Float = (0.0 - t.b) / det
  LET ic AS Float = (0.0 - t.c) / det
  LET id AS Float = t.a / det
  ' The six terms as float32 BIT PATTERNS, whole numbers the emitters copy straight
  ' into the item block. See `__canvas_float32Bits` for why the narrowing happens here
  ' and not in generated code.
  RETURN [toFloat(__canvas_float32Bits(ia)), toFloat(__canvas_float32Bits(ib)), toFloat(__canvas_float32Bits(ic)), toFloat(__canvas_float32Bits(id)), toFloat(__canvas_float32Bits(0.0 - (ia * t.tx + ic * t.ty))), toFloat(__canvas_float32Bits(0.0 - (ib * t.tx + id * t.ty))), 1.0]
END FUNC"#;

/// The forward transform, recovered from the six inverse terms.
///
/// Two callers need it and neither has the original `Transform`: the bounds builder,
/// which maps a shape-space box to the hull the item actually covers, and the glyph
/// blit, which does the same for one glyph's bitmap box. Recovering it beats carrying
/// six more header slots — a bounding box only has to be *conservative*, so the
/// float32 round-trip's last bit cannot matter — and one helper beats two copies of a
/// 2×2 inversion that would eventually disagree.
///
/// Returns `[fa, fb, fc, fd, ftx, fty]`, or the identity if the inverse is somehow
/// singular. It cannot be, in practice: `__canvas_invertTransform` already refused a
/// singular input, and the inverse of an invertible matrix is invertible. The arm is
/// here so this helper cannot be the one place that divides by zero.
#[rustfmt::skip]
const FORWARD_OF: &str =
r#"FUNC __canvas_forwardOf(ia AS Float, ib AS Float, ic AS Float, id AS Float, itx AS Float, ity AS Float) AS List OF Float
  LET det AS Float = ia * id - ib * ic
  IF __canvas_absF(det) < 0.000000000001 THEN
    RETURN [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
  END IF
  LET fa AS Float = id / det
  LET fb AS Float = (0.0 - ib) / det
  LET fc AS Float = (0.0 - ic) / det
  LET fd AS Float = ia / det
  RETURN [fa, fb, fc, fd, 0.0 - (fa * itx + fc * ity), 0.0 - (fb * itx + fd * ity)]
END FUNC"#;

/// plan-172-D: whether a shape's `transform` is in effect.
///
/// The all-zero transform is the identity and so is one with no area (a zero row, or
/// `|a*d - b*c|` under `1e-12`) — the two cases `__canvas_invertTransform` answers with
/// the identity. A shape whose transform is NOT in effect is built at its `position`,
/// with the record every renderer has always drawn for those surface coordinates; one
/// whose transform is in effect is built at the local origin and carried by the affine
/// `__canvas_placement` returns. This function is the single test that picks between
/// them, so the builder and the inverse cannot disagree about which items are turned.
#[rustfmt::skip]
const TRANSFORM_ACTIVE: &str =
r#"FUNC __canvas_transformActive(t AS Transform) AS Boolean
  IF t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0 THEN
    RETURN FALSE
  END IF
  RETURN NOT (__canvas_absF(t.a * t.d - t.b * t.c) < 0.000000000001)
END FUNC"#;

/// plan-172-D: the affine a shape at `position` with `transform` is drawn through.
///
/// A point `p` of the shape's local geometry reaches the surface at
/// `position + transform(p)`: the transform pivots about the reference point, and
/// `transform.tx` / `ty` add to `position`. So the effective affine is the transform's
/// linear part with the translation `position + (tx, ty)`.
///
/// When the transform is not in effect the answer is the all-zero transform, and the
/// caller adds `position` to the geometry's own coordinates instead: building an
/// untransformed shape locally and translating it would send every positioned item
/// down the transformed path — no native builder, no rectangular clip, antialiasing
/// through an inverse map — to draw the same pixels.
#[rustfmt::skip]
const PLACEMENT: &str =
r#"FUNC __canvas_placement(position AS vector::Float2, t AS Transform) AS Transform
  IF NOT __canvas_transformActive(t) THEN
    RETURN __canvas_noTransform()
  END IF
  RETURN Transform[a := t.a, b := t.b, c := t.c, d := t.d, tx := position.x + t.tx, ty := position.y + t.ty]
END FUNC"#;

/// plan-172-D: a paint whose `clip` rings and gradient points are moved by `(ox, oy)`.
///
/// Both are in the shape's local space. A shape built at its position (see
/// `__canvas_placement`) needs them in the same coordinates as its geometry, so they
/// are moved once here and every builder downstream reads them unchanged.
///
/// A paint with no clip and no gradient is returned as it is — the common case, and
/// no allocation. A clip is moved even when the offset is zero: the native builder adds
/// the offset to every clip point unconditionally, and `-0.0 + 0.0` is `+0.0`, so
/// skipping the add here would differ from it in a sign bit. The gradient's two points
/// are moved only when it has the two stops that make it a gradient, which is exactly
/// when the native builder declines the item.
///
/// A gradient's own `transform` is in local space as well, so it is re-centred on the
/// offset with its points. `__canvas_gradientInverse` treats the same transforms as the
/// identity that `__canvas_transformActive` calls not in effect, and those are left
/// alone: the identity is the identity about any point.
#[rustfmt::skip]
const PLACED_PAINT: &str =
r#"FUNC __canvas_placedClip(rings AS List OF List OF vector::Float2, ox AS Float, oy AS Float) AS List OF List OF vector::Float2
  IF len(rings) = 0 THEN
    RETURN rings
  END IF
  MUT out AS List OF List OF vector::Float2 = []
  FOR EACH ring IN rings
    MUT moved AS List OF vector::Float2 = []
    FOR EACH q IN ring
      moved = collections::append(moved, vector::Float2[x := q.x + ox, y := q.y + oy])
    NEXT
    out = collections::append(out, moved)
  NEXT
  RETURN out
END FUNC

FUNC __canvas_placedPaint(paint AS Paint, ox AS Float, oy AS Float) AS Paint
  LET ramp AS Boolean = len(paint.gradient.stops) >= 2
  IF len(paint.clip) = 0 AND NOT ramp THEN
    RETURN paint
  END IF
  MUT g AS Gradient = paint.gradient
  IF ramp THEN
    g = WITH g { startPoint := vector::Float2[x := g.startPoint.x + ox, y := g.startPoint.y + oy], endPoint := vector::Float2[x := g.endPoint.x + ox, y := g.endPoint.y + oy] }
    ' The gradient's own transform G is in local space too. Moved to the build origin
    ' it is G'(x) = G(x - o) + o: the same linear part, and the translation
    ' t + o - L*o. Without this a turned or scaled ramp would be turned about the
    ' surface origin instead of the shape's reference point.
    LET gt AS Transform = g.transform
    IF __canvas_transformActive(gt) THEN
      g = WITH g { transform := Transform[a := gt.a, b := gt.b, c := gt.c, d := gt.d, tx := gt.tx + ox - (gt.a * ox + gt.c * oy), ty := gt.ty + oy - (gt.b * ox + gt.d * oy)] }
    END IF
  END IF
  RETURN WITH paint { clip := __canvas_placedClip(paint.clip, ox, oy), gradient := g }
END FUNC

' plan-172-F: a polygon's rings in the coordinates its record is built in. Its points
' are offsets from `position`; built at its position (no transform in effect) each is
' `position + point`, added whatever the position -- the native builder adds it to
' every point it reads, and `-0.0 + 0.0` is `+0.0`, so skipping the add for a zero
' position would differ from it in a sign bit. A turned polygon is built at the local
' origin, where the points are already what the record wants, and is never built
' natively, so they are returned as they are.
FUNC __canvas_placedRings(rings AS List OF List OF vector::Float2, position AS vector::Float2, t AS Transform) AS List OF List OF vector::Float2
  IF __canvas_transformActive(t) THEN
    RETURN rings
  END IF
  RETURN __canvas_placedClip(rings, position.x, position.y)
END FUNC

FUNC __canvas_buildX(position AS vector::Float2, t AS Transform) AS Float
  IF __canvas_transformActive(t) THEN
    RETURN 0.0
  END IF
  RETURN position.x
END FUNC

FUNC __canvas_buildY(position AS vector::Float2, t AS Transform) AS Float
  IF __canvas_transformActive(t) THEN
    RETURN 0.0
  END IF
  RETURN position.y
END FUNC"#;

/// No clip rings, which means "no clip" (plan-163-C).
#[rustfmt::skip]
const NO_CLIP: &str =
r#"FUNC __canvas_noClip() AS List OF List OF vector::Float2
  RETURN []
END FUNC"#;

/// The all-zero `Gradient`: no stops, so no gradient (plan-116-F); plan-163-D's five
/// fields at their zero values (Pad, the identity, a centred point focus, FillOnly).
///
/// The empty stop list is the no-op, and it is the *only* one that matters — the kind
/// and the two points mean nothing without stops to interpolate between. That is the
/// shape `__canvas_noClip`'s empty ring list already has: a value the renderer
/// tests in one comparison, rather than a sentinel it has to know about.
#[rustfmt::skip]
const NO_GRADIENT: &str =
r#"FUNC __canvas_noGradient() AS Gradient
  RETURN Gradient[kind := GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 0.0, y := 0.0], stops := [], spread := GradientSpread.Pad, transform := __canvas_noTransform(), focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := GradientApply.FillOnly]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_transparent", TRANSPARENT));
    pkg.add_helper(RegistryHelper::always("canvas_noTransform", NO_TRANSFORM));
    pkg.add_helper(RegistryHelper::always("canvas_noClip", NO_CLIP));
    pkg.add_helper(RegistryHelper::always("canvas_noGradient", NO_GRADIENT));
    pkg.add_helper(RegistryHelper::always(
        "canvas_invertTransform",
        INVERT_TRANSFORM,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_float32Bits", FLOAT32_BITS));
    pkg.add_helper(RegistryHelper::always("canvas_forwardOf", FORWARD_OF));
    pkg.add_helper(RegistryHelper::always(
        "canvas_transformActive",
        TRANSFORM_ACTIVE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_placement", PLACEMENT));
    pkg.add_helper(RegistryHelper::always("canvas_placedPaint", PLACED_PAINT));
}
