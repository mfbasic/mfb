//! The zero values the `Paint` constructors fill in for the fields a caller did
//! not name.
//!
//! These exist because **MFBASIC named construction does not default unset
//! fields** — `Paint[fill := c]` is a `TYPE_CONSTRUCTOR_ARITY_MISMATCH`, not a
//! partially-specified record. So `Paint`'s "every field's zero value is that
//! field's no-op" rule is delivered by `canvas::fill`/`stroke`/`fillStroke`
//! writing those zeros explicitly, rather than by the constructor syntax.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Fully transparent — the no-op `Color`, and what an unnamed `Paint` channel is.
#[rustfmt::skip]
const TRANSPARENT: &str =
r#"FUNC __canvas_transparent() AS Color
  RETURN Color[red := toByte(0), green := toByte(0), blue := toByte(0), alpha := toByte(0)]
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
/// **Why this exists here rather than in either emitter.** The item block carries the
/// inverse transform as float32 — 16.16 fixed point holds an inverse term near `0.01`
/// (a 100× scale-up) to about four significant digits, which is roughly ¾ of a pixel of
/// positional error at the far edge of such an item. But the assemblers this compiler
/// emits through have **no double→single convert and no 32-bit float store**
/// (`emit_store_f32_from_integer`'s doc records the same gap), so the narrowing has to
/// be done by hand somewhere. Doing it *here* means one implementation that an ordinary
/// test can check against known bit patterns, instead of two hand-rolled ones in
/// generated machine code where the only symptom of an error is a wrong picture.
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

/// A zero-area `Bounds`, which means "no clip".
#[rustfmt::skip]
const NO_CLIP: &str =
r#"FUNC __canvas_noClip() AS Bounds
  RETURN Bounds[x := 0.0, y := 0.0, w := 0.0, h := 0.0]
END FUNC"#;

/// The all-zero `Gradient`: no stops, so no gradient (plan-116-F).
///
/// The empty stop list is the no-op, and it is the *only* one that matters — the kind
/// and the two points mean nothing without stops to interpolate between. That is the
/// shape `__canvas_noClip`'s zero-area rectangle already has: a value the renderer
/// tests in one comparison, rather than a sentinel it has to know about.
#[rustfmt::skip]
const NO_GRADIENT: &str =
r#"FUNC __canvas_noGradient() AS Gradient
  RETURN Gradient[kind := GradientKind.Linear, startPoint := Point[x := 0.0, y := 0.0], endPoint := Point[x := 0.0, y := 0.0], stops := []]
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
}
