//! `canvas::ParticleSystem` (plan-150-A): one scene item that is drawn as many — a
//! template `DrawItem` placed, turned, scaled and tinted once per live particle, each
//! from a closed-form function of its slot and the system's clock.
//!
//! Two chunks define the same names, as `helper_autopaint.rs`'s do: [`BODY`] for a
//! program that constructs a `ParticleSystem` ([`HelperGate::WhenUsedOrConstructed`]) and
//! [`ABSENT`] for one that does not ([`HelperGate::WhenNotConstructed`]). The item unions
//! have the variant in every canvas program, so every `MATCH` over them has its arm, and
//! each arm calls a hook; a program with no system carries the hooks' empty bodies and
//! none of the model. A `ParticleSystem` holds a union field, so it has no default value
//! and constructing one is the only way to hold one — which is why the constructed gate
//! is the right one here, as it is for `AutoPaint` (`.ai/canvas-threading.md` section 16).

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

/// The record type whose construction opens this chunk.
pub(super) const CONSTRUCTED: &[&str] = &["ParticleSystem"];

/// The analytic model (plan-150-A section 5), as functions of numbers and points alone:
/// no record of the type set, no global but the handful a function answers a second
/// value through, no clock and no drawing. Every particle's state is a closed form in
/// its slot and the system's `time`, so the whole model is testable as arithmetic —
/// `tests/canvas/rt_canvas_particles.rs` reads this text out of this file, compiles it
/// behind a driver, and runs it — and [`EXPAND`] is what reads a `ParticleSystem`'s
/// fields into it.
///
/// **Nothing here may raise.** It runs on the graphics thread, where a `FAIL` has no
/// user frame to unwind to, so every conversion to an integer is guarded: a value that
/// is not a number, or is too large to mean anything, makes the slot dead or the angle
/// zero instead.
///
/// **The trigonometry is the package's own** (`helper_shapes.rs`), not `math::sin`,
/// and the exponential is written out here, for the reason the rasteriser is written in
/// MFBASIC at all: the picture for a `(seed, time)` has to be the same on every target,
/// and a platform's `libm` is not.
#[rustfmt::skip]
pub(super) const MODEL: &str =
r#"LET __CANVAS_PARTICLE_M31 AS Integer = 2147483647

' One round of mixing over 31 bits: xor-shift, multiply, twice. The products stay
' under 2^58, so nothing here can overflow.
FUNC __canvas_particleMix(x AS Integer) AS Integer
  MUT h AS Integer = bits::band(x, __CANVAS_PARTICLE_M31)
  h = bits::bxor(h, bits::sr(h, 16))
  h = bits::band(h * 73244475, __CANVAS_PARTICLE_M31)
  h = bits::bxor(h, bits::sr(h, 15))
  h = bits::band(h * 73244475, __CANVAS_PARTICLE_M31)
  RETURN bits::bxor(h, bits::sr(h, 16))
END FUNC

' One random draw in [0, 1) for a slot: a function of the system's seed, the slot, which
' life of the slot this is (`cycle`, so a slot born again does not repeat itself) and
' which draw it is (`stream`, so two draws for one particle are unrelated).
'
' Deliberately NOT the item hash's step (`__canvas_hashStep`). That one is a pair of
' multiply-and-add lanes, which is what a cache key wants and is affine in its input:
' two streams of it differ by a constant for every slot, so a particle's two emitter
' coordinates would fall on a line.
FUNC __canvas_particleHash(seed AS Integer, slot AS Integer, cycle AS Integer, stream AS Integer) AS Float
  MUT h AS Integer = __canvas_particleMix(bits::bxor(bits::band(seed, __CANVAS_PARTICLE_M31), bits::band(bits::sr(seed, 31), __CANVAS_PARTICLE_M31)))
  h = __canvas_particleMix(bits::bxor(h, bits::band(slot, __CANVAS_PARTICLE_M31)) + 1013904223)
  h = __canvas_particleMix(bits::bxor(h, bits::band(cycle, __CANVAS_PARTICLE_M31)) + 1664525)
  h = __canvas_particleMix(bits::bxor(h, bits::band(stream, __CANVAS_PARTICLE_M31)) + 12345)
  RETURN toFloat(h) / 2147483648.0
END FUNC

' A `...Var` fraction: 0 to 1, a value outside it taken as the nearer end and one that
' is not a number as 0.
FUNC __canvas_particleClamp01(v AS Float) AS Float
  IF NOT (v > 0.0) THEN
    RETURN 0.0
  END IF
  IF v > 1.0 THEN
    RETURN 1.0
  END IF
  RETURN v
END FUNC

' `__canvas_particleAge`'s second answer: which life of the slot this is.
MUT __CANVAS_PARTICLE_CYCLE AS Integer = 0

' How long ago the particle a slot holds was born, in seconds -- or -1.0 when the slot
' holds none: before its first birth, outside the pool or the burst, or in a system that
' emits nothing.
'
'   Continuous   the pool is born one slot after another, `rate` a second, and round
'                again: slot i is born at i / rate and every maxParticles / rate after.
'   Burst        slots 0 to burst - 1 are born together at time 0, and again every
'                burstPeriod when that is above zero.
FUNC __canvas_particleAge(burstMode AS Boolean, maxParticles AS Integer, rate AS Float, burst AS Integer, burstPeriod AS Float, slot AS Integer, time AS Float) AS Float
  __CANVAS_PARTICLE_CYCLE = 0
  IF NOT (time >= 0.0) OR slot < 0 OR slot >= maxParticles THEN
    RETURN 0.0 - 1.0
  END IF
  IF burstMode THEN
    IF slot >= burst THEN
      RETURN 0.0 - 1.0
    END IF
    IF NOT (burstPeriod > 0.0) THEN
      RETURN time
    END IF
    LET bursts AS Float = time / burstPeriod
    IF NOT (bursts < 1000000000000.0) THEN
      RETURN 0.0 - 1.0
    END IF
    __CANVAS_PARTICLE_CYCLE = toInt(bursts)
    RETURN time - toFloat(__CANVAS_PARTICLE_CYCLE) * burstPeriod
  END IF
  IF NOT (rate > 0.0) THEN
    RETURN 0.0 - 1.0
  END IF
  LET since AS Float = time - toFloat(slot) / rate
  IF NOT (since >= 0.0) THEN
    RETURN 0.0 - 1.0
  END IF
  LET period AS Float = toFloat(maxParticles) / rate
  LET rounds AS Float = since / period
  IF NOT (rounds < 1000000000000.0) THEN
    RETURN 0.0 - 1.0
  END IF
  __CANVAS_PARTICLE_CYCLE = toInt(rounds)
  RETURN since - toFloat(__CANVAS_PARTICLE_CYCLE) * period
END FUNC

' How long one particle lives: `lifetime`, more or less its share of `lifetimeVar`.
FUNC __canvas_particleLife(lifetime AS Float, lifetimeVar AS Float, r AS Float) AS Float
  RETURN lifetime * (1.0 + __canvas_particleClamp01(lifetimeVar) * (2.0 * r - 1.0))
END FUNC

' An angle brought into -PI..PI in one step. The package's sine wraps by repeated
' subtraction, which is right for the angles an item is built with and would not end
' for a particle that has been spinning for an hour.
FUNC __canvas_particleWrap(angle AS Float) AS Float
  IF NOT (angle > 0.0 - 1000000000000.0 AND angle < 1000000000000.0) THEN
    RETURN 0.0
  END IF
  RETURN angle - 6.283185307179586 * math::floor(angle / 6.283185307179586 + 0.5)
END FUNC

' The unit vector at `angle`, in `__CANVAS_PARTICLE_UX` / `_UY`: clockwise from +X under
' the canvas' Y-down axes, as an `Arc`'s angles are. Normalised, because the package's
' sine and cosine are good to a few parts in ten million and a point meant to lie ON a
' circle should.
MUT __CANVAS_PARTICLE_UX AS Float = 1.0
MUT __CANVAS_PARTICLE_UY AS Float = 0.0

FUNC __canvas_particleUnit(angle AS Float) AS Nothing
  LET wrapped AS Float = __canvas_particleWrap(angle)
  LET c AS Float = __canvas_cos(wrapped)
  LET s AS Float = __canvas_sin(wrapped)
  LET n AS Float = math::sqrt(c * c + s * s)
  __CANVAS_PARTICLE_UX = c / n
  __CANVAS_PARTICLE_UY = s / n
END FUNC

' Where a sampler below put its point, in the emitter's LOCAL geometry -- the space
' every shape is built in, with its reference point at (0, 0).
MUT __CANVAS_PARTICLE_X AS Float = 0.0
MUT __CANVAS_PARTICLE_Y AS Float = 0.0

' A point of a disc, or of an ellipse with these radii, uniform over its AREA. The
' square root is what makes it so: a radius drawn evenly would put as many points in
' the inner half-radius disc as in the ring around it, which has three times the area.
FUNC __canvas_particleDisc(radiusX AS Float, radiusY AS Float, r1 AS Float, r2 AS Float) AS Nothing
  LET r AS Float = math::sqrt(r1)
  __canvas_particleUnit(6.283185307179586 * r2)
  __CANVAS_PARTICLE_X = radiusX * r * __CANVAS_PARTICLE_UX
  __CANVAS_PARTICLE_Y = radiusY * r * __CANVAS_PARTICLE_UY
END FUNC

' A point of a box, uniform over its area: from its top-left corner, or about its
' centre.
FUNC __canvas_particleBox(width AS Float, height AS Float, centred AS Boolean, r1 AS Float, r2 AS Float) AS Nothing
  __CANVAS_PARTICLE_X = r1 * width
  __CANVAS_PARTICLE_Y = r2 * height
  IF centred THEN
    __CANVAS_PARTICLE_X = __CANVAS_PARTICLE_X - width / 2.0
    __CANVAS_PARTICLE_Y = __CANVAS_PARTICLE_Y - height / 2.0
  END IF
END FUNC

' A point of a segment, uniform along it.
FUNC __canvas_particleSegment(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, r1 AS Float) AS Nothing
  __CANVAS_PARTICLE_X = x0 + (x1 - x0) * r1
  __CANVAS_PARTICLE_Y = y0 + (y1 - y0) * r1
END FUNC

' A point of a circular arc, uniform along it.
FUNC __canvas_particleArc(radius AS Float, startAngle AS Float, endAngle AS Float, r1 AS Float) AS Nothing
  __canvas_particleUnit(startAngle + (endAngle - startAngle) * r1)
  __CANVAS_PARTICLE_X = radius * __CANVAS_PARTICLE_UX
  __CANVAS_PARTICLE_Y = radius * __CANVAS_PARTICLE_UY
END FUNC

' A point of an open path through `points`, uniform along its length; closed back to
' its first point when `closed`. FALSE when there is no point to stand on.
FUNC __canvas_particleAlong(points AS List OF vector::Float2, closed AS Boolean, r1 AS Float) AS Boolean
  LET count AS Integer = len(points)
  IF count = 0 THEN
    RETURN FALSE
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET first AS vector::Float2 = collections::getOr(points, 0, origin)
  MUT edges AS Integer = count - 1
  IF closed THEN
    edges = count
  END IF
  MUT total AS Float = 0.0
  MUT i AS Integer = 0
  WHILE i < edges
    LET a AS vector::Float2 = collections::getOr(points, i, origin)
    LET b AS vector::Float2 = collections::getOr(points, (i + 1) MOD count, origin)
    total = total + math::sqrt((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y))
    i = i + 1
  END WHILE
  __CANVAS_PARTICLE_X = first.x
  __CANVAS_PARTICLE_Y = first.y
  IF NOT (total > 0.0) THEN
    RETURN TRUE
  END IF
  MUT left AS Float = total * r1
  i = 0
  WHILE i < edges
    LET a AS vector::Float2 = collections::getOr(points, i, origin)
    LET b AS vector::Float2 = collections::getOr(points, (i + 1) MOD count, origin)
    LET length AS Float = math::sqrt((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y))
    IF length > 0.0 THEN
      ' The last edge takes whatever rounding left over, so the point never falls off
      ' the end of the path.
      IF left <= length OR i = edges - 1 THEN
        MUT f AS Float = left / length
        IF f > 1.0 THEN
          f = 1.0
        END IF
        __CANVAS_PARTICLE_X = a.x + (b.x - a.x) * f
        __CANVAS_PARTICLE_Y = a.y + (b.y - a.y) * f
        RETURN TRUE
      END IF
      left = left - length
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC

' Whether a point is inside closed rings under a fill rule: the crossings of a ray to
' the right, counted for even-odd and signed for non-zero -- the rule a `Polygon` is
' filled by.
FUNC __canvas_particleInRings(rings AS List OF List OF vector::Float2, nonZero AS Boolean, x AS Float, y AS Float) AS Boolean
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  MUT crossings AS Integer = 0
  MUT winding AS Integer = 0
  FOR EACH ring IN rings
    LET count AS Integer = len(ring)
    MUT i AS Integer = 0
    WHILE i < count AND count >= 3
      LET a AS vector::Float2 = collections::getOr(ring, i, origin)
      LET b AS vector::Float2 = collections::getOr(ring, (i + 1) MOD count, origin)
      IF (a.y <= y) <> (b.y <= y) THEN
        LET at AS Float = a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y)
        IF x < at THEN
          crossings = crossings + 1
          IF b.y > a.y THEN
            winding = winding + 1
          ELSE
            winding = winding - 1
          END IF
        END IF
      END IF
      i = i + 1
    END WHILE
  NEXT
  IF nonZero THEN
    RETURN winding <> 0
  END IF
  RETURN crossings MOD 2 = 1
END FUNC

' How many candidates a slot may try for a point inside a polygon.
LET __CANVAS_PARTICLE_TRIES AS Integer = 32

' A point inside closed rings, uniform over the area the fill rule fills: candidates
' drawn evenly in the rings' bounding box, the first one inside kept. Bounded, and the
' candidates come from the slot's own draws (streams 16 up, clear of every other), so
' the answer is as repeatable as any other. FALSE when no candidate was inside -- a
' polygon that fills almost none of its box -- and the slot then holds no particle for
' that life.
FUNC __canvas_particleInside(rings AS List OF List OF vector::Float2, nonZero AS Boolean, seed AS Integer, slot AS Integer, cycle AS Integer) AS Boolean
  MUT any AS Boolean = FALSE
  MUT minX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT maxY AS Float = 0.0
  FOR EACH ring IN rings
    FOR EACH q IN ring
      IF NOT any THEN
        minX = q.x
        maxX = q.x
        minY = q.y
        maxY = q.y
        any = TRUE
      ELSE
        IF q.x < minX THEN
          minX = q.x
        END IF
        IF q.x > maxX THEN
          maxX = q.x
        END IF
        IF q.y < minY THEN
          minY = q.y
        END IF
        IF q.y > maxY THEN
          maxY = q.y
        END IF
      END IF
    NEXT
  NEXT
  IF NOT any THEN
    RETURN FALSE
  END IF
  MUT k AS Integer = 0
  WHILE k < __CANVAS_PARTICLE_TRIES
    LET x AS Float = minX + (maxX - minX) * __canvas_particleHash(seed, slot, cycle, 16 + k * 2)
    LET y AS Float = minY + (maxY - minY) * __canvas_particleHash(seed, slot, cycle, 17 + k * 2)
    IF __canvas_particleInRings(rings, nonZero, x, y) THEN
      __CANVAS_PARTICLE_X = x
      __CANVAS_PARTICLE_Y = y
      RETURN TRUE
    END IF
    k = k + 1
  END WHILE
  RETURN FALSE
END FUNC

' The sampled point carried from the emitter's local geometry to the surface, the way a
' point of that shape is when it is drawn: through the emitter's transform when it is
' in effect (`active`), then to the emitter's position, measured from the system's.
FUNC __canvas_particlePlace(systemX AS Float, systemY AS Float, emitterX AS Float, emitterY AS Float, active AS Boolean, a AS Float, b AS Float, c AS Float, d AS Float, tx AS Float, ty AS Float) AS Nothing
  LET x AS Float = __CANVAS_PARTICLE_X
  LET y AS Float = __CANVAS_PARTICLE_Y
  IF active THEN
    __CANVAS_PARTICLE_X = systemX + emitterX + a * x + c * y + tx
    __CANVAS_PARTICLE_Y = systemY + emitterY + b * x + d * y + ty
  ELSE
    __CANVAS_PARTICLE_X = systemX + emitterX + x
    __CANVAS_PARTICLE_Y = systemY + emitterY + y
  END IF
END FUNC

' e to the minus x, for x of zero or more: x is split into a whole number of halvings
' and a remainder under 0.35 either side of zero, whose series converges in a dozen
' terms.
FUNC __canvas_particleExpNeg(x AS Float) AS Float
  IF NOT (x > 0.0) THEN
    RETURN 1.0
  END IF
  IF x > 745.0 THEN
    RETURN 0.0
  END IF
  LET halvings AS Integer = toInt(x / 0.6931471805599453 + 0.5)
  LET r AS Float = toFloat(halvings) * 0.6931471805599453 - x
  MUT term AS Float = 1.0
  MUT sum AS Float = 1.0
  MUT n AS Integer = 1
  WHILE n <= 13
    term = term * r / toFloat(n)
    sum = sum + term
    n = n + 1
  END WHILE
  MUT scale AS Float = 1.0
  MUT base AS Float = 0.5
  MUT k AS Integer = halvings
  WHILE k > 0
    IF k MOD 2 = 1 THEN
      scale = scale * base
    END IF
    base = base * base
    k = k / 2
  END WHILE
  RETURN sum * scale
END FUNC

' One axis of a particle's path: where it is `t` seconds after leaving `p0` at `v0`
' under a steady pull `g` and a drag `k`.
'
' This is the exact solution of linear drag,
'
'     p = p0 + (v0 - g/k) (1 - e^(-kt)) / k + (g/k) t
'
' written as p0 + v0 t phi + g t^2 psi with phi = (1 - e^(-x)) / x,
' psi = (x - 1 + e^(-x)) / x^2 and x = kt. The first form divides by k twice and loses
' every digit as k goes to zero; phi and psi go to 1 and 1/2 there, which is the
' no-drag parabola, and are taken from their series while x is small. So drag has no
' threshold at which the path changes shape. A drag of zero or less is none.
FUNC __canvas_particlePosition(p0 AS Float, v0 AS Float, g AS Float, k AS Float, t AS Float) AS Float
  LET x AS Float = k * t
  MUT phi AS Float = 1.0
  MUT psi AS Float = 0.5
  IF x > 0.0 THEN
    IF x < 0.25 THEN
      ' phi = sum (-x)^n / (n+1)!, psi = sum (-x)^n / (n+2)!
      MUT pTerm AS Float = 1.0
      MUT sTerm AS Float = 0.5
      MUT n AS Integer = 1
      WHILE n <= 12
        pTerm = pTerm * (0.0 - x) / toFloat(n + 1)
        sTerm = sTerm * (0.0 - x) / toFloat(n + 2)
        phi = phi + pTerm
        psi = psi + sTerm
        n = n + 1
      END WHILE
    ELSE
      LET e AS Float = __canvas_particleExpNeg(x)
      phi = (1.0 - e) / x
      psi = (x - 1.0 + e) / (x * x)
    END IF
  END IF
  RETURN p0 + v0 * t * phi + g * t * t * psi
END FUNC

' The wander added to one axis: a sine of the particle's own phase.
FUNC __canvas_particleWander(amplitude AS Float, hz AS Float, t AS Float, r AS Float) AS Float
  IF amplitude = 0.0 THEN
    RETURN 0.0
  END IF
  RETURN amplitude * __canvas_sin(__canvas_particleWrap(6.283185307179586 * (hz * t + r)))
END FUNC

' How far a particle is turned: where it started, and its spin since.
FUNC __canvas_particleTurn(rotation AS Float, rotationVar AS Float, spin AS Float, spinVar AS Float, t AS Float, r5 AS Float, r6 AS Float) AS Float
  RETURN rotation + rotationVar * (2.0 * r5 - 1.0) + spin * (1.0 + __canvas_particleClamp01(spinVar) * (2.0 * r6 - 1.0)) * t
END FUNC

' How big a particle is at `u` of the way through its life.
FUNC __canvas_particleSize(startScale AS Float, endScale AS Float, scaleVar AS Float, u AS Float, r7 AS Float) AS Float
  RETURN (startScale + (endScale - startScale) * u) * (1.0 + __canvas_particleClamp01(scaleVar) * (2.0 * r7 - 1.0))
END FUNC"#;

/// What reads a `ParticleSystem` into [`MODEL`]: the emitter prepared once per system
/// per frame, and one particle's state from its slot.
///
/// **The emitter is prepared, not re-read per particle.** A `PolyCurve` is flattened, a
/// `Text` is shaped and measured, a `Polygon`'s rings are copied out of the item — once,
/// into the globals below — and every slot of the system then samples from those. They
/// are graphics-thread globals, like everything the frame's walk keeps
/// (`.ai/canvas-threading.md` section 2).
#[rustfmt::skip]
const EXPAND: &str =
r#"' The prepared emitter. `__CANVAS_PARTICLE_EKIND` picks the sampler: 0 nothing to emit
' from, 1 a disc or ellipse (E0, E1 its radii), 2 a box (E0 by E1, its corner at
' E2, E3), 3 a segment (E0, E1 to E2, E3), 4 an arc (radius E0, E1 to E2), 5 a path
' along `__CANVAS_PARTICLE_EPOINTS`, 6 the inside of `__CANVAS_PARTICLE_ERINGS`.
MUT __CANVAS_PARTICLE_EKIND AS Integer = 0
MUT __CANVAS_PARTICLE_E0 AS Float = 0.0
MUT __CANVAS_PARTICLE_E1 AS Float = 0.0
MUT __CANVAS_PARTICLE_E2 AS Float = 0.0
MUT __CANVAS_PARTICLE_E3 AS Float = 0.0
MUT __CANVAS_PARTICLE_ECLOSED AS Boolean = FALSE
MUT __CANVAS_PARTICLE_ENONZERO AS Boolean = FALSE
MUT __CANVAS_PARTICLE_EPOINTS AS List OF vector::Float2 = []
MUT __CANVAS_PARTICLE_ERINGS AS List OF List OF vector::Float2 = []
' Where the emitter is and how it is turned: the system's position, the emitter's own,
' and the emitter's transform when it is in effect.
MUT __CANVAS_PARTICLE_SX AS Float = 0.0
MUT __CANVAS_PARTICLE_SY AS Float = 0.0
MUT __CANVAS_PARTICLE_EX AS Float = 0.0
MUT __CANVAS_PARTICLE_EY AS Float = 0.0
MUT __CANVAS_PARTICLE_EACTIVE AS Boolean = FALSE
MUT __CANVAS_PARTICLE_ET AS Transform = Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0]

FUNC __canvas_particlePlacedAt(position AS vector::Float2, t AS Transform) AS Nothing
  __CANVAS_PARTICLE_EX = position.x
  __CANVAS_PARTICLE_EY = position.y
  __CANVAS_PARTICLE_EACTIVE = __canvas_transformActive(t)
  __CANVAS_PARTICLE_ET = t
END FUNC

' A curve chain's outline as straight edges, in the shape's local geometry: the
' flattening a `Picture` is cut to (`__canvas_curveRings`), asked of the same curves
' with no position and no transform.
FUNC __canvas_particleCurveRings(contours AS List OF List OF BezierCurve, rule AS FillRule) AS List OF List OF vector::Float2
  RETURN __canvas_curveRings(CurvePolygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := contours, rule := rule, paint := Paint[fill := __canvas_transparent(), stroke := __canvas_transparent(), strokeWidth := 0.0, blend := BlendMode.Normal, clip := __canvas_noClip(), clipRule := FillRule.EvenOdd, gradient := __canvas_noGradient(), cap := CapStyle.Round, join := JoinStyle.Round, miterLimit := 0.0, dashes := [], dashOffset := 0.0, transparency := 0.0], transform := __canvas_noTransform()])
END FUNC

' A `Text` emitter: the box around its line -- its advance wide, from its ascent above
' the baseline to its descent below -- which is the box a `Text` is hit on. A font that
' is closed, or a run with no glyph, emits nothing.
FUNC __canvas_particleTextBox(t AS Text) AS Nothing
  LET font AS Integer = canvas::fontHandle(t.font)
  LET b AS List OF Byte = __canvas_fontBlob(font)
  IF len(b) = 0 THEN
    RETURN
  END IF
  LET upem AS Integer = __canvas_fontUnitsPerEm(b)
  IF upem <= 0 THEN
    RETURN
  END IF
  LET shaped AS List OF Integer = __canvas_shapeRun([font], t.text)
  IF len(shaped) < 3 THEN
    RETURN
  END IF
  LET scale AS Float = t.fontSize / toFloat(upem)
  MUT advance AS Float = 0.0
  MUT c AS Integer = 0
  WHILE c * 3 < len(shaped)
    advance = advance + toFloat(collections::getOr(shaped, c * 3 + 2, 0)) * scale
    c = c + 1
  END WHILE
  LET up AS Float = toFloat(__canvas_fontAscent(b)) * scale
  LET down AS Float = toFloat(0 - __canvas_fontDescent(b)) * scale
  __CANVAS_PARTICLE_EKIND = 2
  __CANVAS_PARTICLE_E0 = advance
  __CANVAS_PARTICLE_E1 = up + down
  __CANVAS_PARTICLE_E2 = 0.0
  __CANVAS_PARTICLE_E3 = 0.0 - up
END FUNC

' Read a system's emitter into the globals above. Once per system per frame.
FUNC __canvas_particlePrepare(ps AS ParticleSystem) AS Nothing
  __CANVAS_PARTICLE_EKIND = 0
  __CANVAS_PARTICLE_EPOINTS = []
  __CANVAS_PARTICLE_ERINGS = []
  __CANVAS_PARTICLE_ECLOSED = FALSE
  __CANVAS_PARTICLE_ENONZERO = FALSE
  __CANVAS_PARTICLE_SX = ps.position.x
  __CANVAS_PARTICLE_SY = ps.position.y
  MATCH ps.emitter
    CASE Circle(c)
      __CANVAS_PARTICLE_EKIND = 1
      __CANVAS_PARTICLE_E0 = c.radius
      __CANVAS_PARTICLE_E1 = c.radius
      __canvas_particlePlacedAt(c.position, c.transform)
    CASE Ellipse(e)
      __CANVAS_PARTICLE_EKIND = 1
      __CANVAS_PARTICLE_E0 = e.radiusX
      __CANVAS_PARTICLE_E1 = e.radiusY
      __canvas_particlePlacedAt(e.position, e.transform)
    ' The box, whatever its corner radii.
    CASE Rectangle(r)
      __CANVAS_PARTICLE_EKIND = 2
      __CANVAS_PARTICLE_E0 = r.size.width
      __CANVAS_PARTICLE_E1 = r.size.height
      __CANVAS_PARTICLE_E2 = 0.0
      __CANVAS_PARTICLE_E3 = 0.0
      IF r.bind = BindTo.Center THEN
        __CANVAS_PARTICLE_E2 = 0.0 - r.size.width / 2.0
        __CANVAS_PARTICLE_E3 = 0.0 - r.size.height / 2.0
      END IF
      __canvas_particlePlacedAt(r.position, r.transform)
    CASE Line(l)
      __CANVAS_PARTICLE_EKIND = 3
      __CANVAS_PARTICLE_E0 = l.startAt.x
      __CANVAS_PARTICLE_E1 = l.startAt.y
      __CANVAS_PARTICLE_E2 = l.endAt.x
      __CANVAS_PARTICLE_E3 = l.endAt.y
      __canvas_particlePlacedAt(l.position, l.transform)
    CASE Arc(a)
      __CANVAS_PARTICLE_EKIND = 4
      __CANVAS_PARTICLE_E0 = a.radius
      __CANVAS_PARTICLE_E1 = a.startAngle
      __CANVAS_PARTICLE_E2 = a.endAngle
      __canvas_particlePlacedAt(a.position, a.transform)
    ' The path, whatever its colours.
    CASE PolyLine(pl)
      __CANVAS_PARTICLE_EKIND = 5
      __CANVAS_PARTICLE_EPOINTS = pl.points
      __canvas_particlePlacedAt(pl.position, pl.transform)
    CASE Polygon(p)
      __CANVAS_PARTICLE_EKIND = 6
      __CANVAS_PARTICLE_ERINGS = p.contours
      __CANVAS_PARTICLE_ENONZERO = p.rule = FillRule.NonZero
      __canvas_particlePlacedAt(p.position, p.transform)
    CASE Text(t)
      __canvas_particleTextBox(t)
      __canvas_particlePlacedAt(t.position, t.transform)
    ' A chain of curves, flattened: one open path -- or a closed one, when it ends on
    ' the point it began on, which the flattening leaves out.
    CASE PolyCurve(pc)
      LET none AS List OF vector::Float2 = []
      LET chain AS List OF vector::Float2 = collections::getOr(__canvas_particleCurveRings([pc.curves], FillRule.EvenOdd), 0, none)
      __CANVAS_PARTICLE_EKIND = 5
      __CANVAS_PARTICLE_EPOINTS = chain
      __CANVAS_PARTICLE_ECLOSED = __canvas_particleChainCloses(pc.curves)
      __canvas_particlePlacedAt(pc.position, pc.transform)
    CASE CurvePolygon(cp)
      __CANVAS_PARTICLE_EKIND = 6
      __CANVAS_PARTICLE_ERINGS = __canvas_particleCurveRings(cp.contours, cp.rule)
      __CANVAS_PARTICLE_ENONZERO = cp.rule = FillRule.NonZero
      __canvas_particlePlacedAt(cp.position, cp.transform)
  END MATCH
END FUNC

' Whether a chain of curves ends on the point it began on.
FUNC __canvas_particleChainCloses(curves AS List OF BezierCurve) AS Boolean
  IF len(curves) = 0 THEN
    RETURN FALSE
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET none AS BezierCurve = BezierCurve[points := [], weights := []]
  LET head AS BezierCurve = collections::getOr(curves, 0, none)
  LET tail AS BezierCurve = collections::getOr(curves, len(curves) - 1, none)
  IF len(head.points) = 0 OR len(tail.points) = 0 THEN
    RETURN FALSE
  END IF
  LET a AS vector::Float2 = collections::getOr(head.points, 0, origin)
  LET b AS vector::Float2 = collections::getOr(tail.points, len(tail.points) - 1, origin)
  RETURN a.x = b.x AND a.y = b.y
END FUNC

' Where the particle in a slot was born, on the surface, in `__CANVAS_PARTICLE_X` /
' `_Y`, from the emitter `__canvas_particlePrepare` read. FALSE when the emitter has
' nowhere to put it.
FUNC __canvas_particleSpawn(seed AS Integer, slot AS Integer, cycle AS Integer) AS Boolean
  LET kind AS Integer = __CANVAS_PARTICLE_EKIND
  IF kind = 0 THEN
    RETURN FALSE
  END IF
  LET r1 AS Float = __canvas_particleHash(seed, slot, cycle, 1)
  IF kind = 1 THEN
    __canvas_particleDisc(__CANVAS_PARTICLE_E0, __CANVAS_PARTICLE_E1, r1, __canvas_particleHash(seed, slot, cycle, 2))
  ELSEIF kind = 2 THEN
    __canvas_particleBox(__CANVAS_PARTICLE_E0, __CANVAS_PARTICLE_E1, FALSE, r1, __canvas_particleHash(seed, slot, cycle, 2))
    __CANVAS_PARTICLE_X = __CANVAS_PARTICLE_X + __CANVAS_PARTICLE_E2
    __CANVAS_PARTICLE_Y = __CANVAS_PARTICLE_Y + __CANVAS_PARTICLE_E3
  ELSEIF kind = 3 THEN
    __canvas_particleSegment(__CANVAS_PARTICLE_E0, __CANVAS_PARTICLE_E1, __CANVAS_PARTICLE_E2, __CANVAS_PARTICLE_E3, r1)
  ELSEIF kind = 4 THEN
    __canvas_particleArc(__CANVAS_PARTICLE_E0, __CANVAS_PARTICLE_E1, __CANVAS_PARTICLE_E2, r1)
  ELSEIF kind = 5 THEN
    IF NOT __canvas_particleAlong(__CANVAS_PARTICLE_EPOINTS, __CANVAS_PARTICLE_ECLOSED, r1) THEN
      RETURN FALSE
    END IF
  ELSE
    IF NOT __canvas_particleInside(__CANVAS_PARTICLE_ERINGS, __CANVAS_PARTICLE_ENONZERO, seed, slot, cycle) THEN
      RETURN FALSE
    END IF
  END IF
  LET t AS Transform = __CANVAS_PARTICLE_ET
  __canvas_particlePlace(__CANVAS_PARTICLE_SX, __CANVAS_PARTICLE_SY, __CANVAS_PARTICLE_EX, __CANVAS_PARTICLE_EY, __CANVAS_PARTICLE_EACTIVE, t.a, t.b, t.c, t.d, t.tx, t.ty)
  RETURN TRUE
END FUNC

' One particle: FALSE when the slot holds none at the system's `time`, otherwise where
' it is (`__CANVAS_PARTICLE_PX` / `_PY`), how far it is turned (`_ROT`, radians), how
' big it is (`_SIZE`) and how far through its life (`_U`, 0 up to 1).
'
' The draws, by stream: 0 lifetime, 1 and 2 the emitter sample, 3 direction, 4 speed,
' 5 rotation, 6 spin, 7 size, 8 and 9 the wander's phase on X and on Y. (A polygon
' emitter's candidates are 16 up.)
MUT __CANVAS_PARTICLE_PX AS Float = 0.0
MUT __CANVAS_PARTICLE_PY AS Float = 0.0
MUT __CANVAS_PARTICLE_ROT AS Float = 0.0
MUT __CANVAS_PARTICLE_SIZE AS Float = 1.0
MUT __CANVAS_PARTICLE_U AS Float = 0.0

FUNC __canvas_particleState(ps AS ParticleSystem, slot AS Integer) AS Boolean
  LET e AS Emission = ps.emission
  LET age AS Float = __canvas_particleAge(e.mode = EmissionMode.Burst, e.maxParticles, e.rate, e.burst, e.burstPeriod, slot, ps.time)
  IF age < 0.0 THEN
    RETURN FALSE
  END IF
  LET cycle AS Integer = __CANVAS_PARTICLE_CYCLE
  LET seed AS Integer = ps.seed
  LET life AS Float = __canvas_particleLife(e.lifetime, e.lifetimeVar, __canvas_particleHash(seed, slot, cycle, 0))
  IF NOT (age < life) THEN
    RETURN FALSE
  END IF
  IF NOT __canvas_particleSpawn(seed, slot, cycle) THEN
    RETURN FALSE
  END IF
  LET m AS Motion = ps.motion
  __canvas_particleUnit(m.angle + m.spread * (2.0 * __canvas_particleHash(seed, slot, cycle, 3) - 1.0))
  LET speed AS Float = m.speed * (1.0 + __canvas_particleClamp01(m.speedVar) * (2.0 * __canvas_particleHash(seed, slot, cycle, 4) - 1.0))
  __CANVAS_PARTICLE_PX = __canvas_particlePosition(__CANVAS_PARTICLE_X, speed * __CANVAS_PARTICLE_UX, m.gravityX, m.drag, age) + __canvas_particleWander(m.turbulence, m.turbulenceHz, age, __canvas_particleHash(seed, slot, cycle, 8))
  __CANVAS_PARTICLE_PY = __canvas_particlePosition(__CANVAS_PARTICLE_Y, speed * __CANVAS_PARTICLE_UY, m.gravityY, m.drag, age) + __canvas_particleWander(m.turbulence, m.turbulenceHz, age, __canvas_particleHash(seed, slot, cycle, 9))
  __CANVAS_PARTICLE_ROT = __canvas_particleTurn(m.rotation, m.rotationVar, m.spin, m.spinVar, age, __canvas_particleHash(seed, slot, cycle, 5), __canvas_particleHash(seed, slot, cycle, 6))
  __CANVAS_PARTICLE_U = age / life
  LET a AS Appearance = ps.appearance
  __CANVAS_PARTICLE_SIZE = __canvas_particleSize(a.startScale, a.endScale, a.scaleVar, __CANVAS_PARTICLE_U, __canvas_particleHash(seed, slot, cycle, 7))
  RETURN TRUE
END FUNC"#;

/// What a `ParticleSystem` is to the walks over a scene.
#[rustfmt::skip]
const HOOKS_BODY: &str =
r#"' A system's template as the `DrawItem` it would be on its own. An `AutoShape` shares
' every tag it has with `DrawItem`, but a union that INCLUDES another is not that union:
' the value has to be rebuilt as the wider one before a walk over `DrawItem` can take it.
'
' It copies the template, once per call -- per system per frame, never per particle.
FUNC __canvas_particleTemplate(t AS AutoShape) AS DrawItem
  MATCH t
    CASE Rectangle(r)
      LET item AS DrawItem = r
      RETURN item
    CASE Line(l)
      LET item AS DrawItem = l
      RETURN item
    CASE Polygon(p)
      LET item AS DrawItem = p
      RETURN item
    CASE Circle(c)
      LET item AS DrawItem = c
      RETURN item
    CASE Arc(arc)
      LET item AS DrawItem = arc
      RETURN item
    CASE Text(tx)
      LET item AS DrawItem = tx
      RETURN item
    CASE Ellipse(e)
      LET item AS DrawItem = e
      RETURN item
    CASE PolyLine(pl)
      LET item AS DrawItem = pl
      RETURN item
    CASE PolyCurve(pc)
      LET item AS DrawItem = pc
      RETURN item
    CASE CurvePolygon(cp)
      LET item AS DrawItem = cp
      RETURN item
    CASE AutoPaint(a)
      LET item AS DrawItem = a
      RETURN item
  END MATCH
  RETURN __canvas_noItem()
END FUNC

' A system's emitter as the `DrawItem` it would be on its own -- the same rebuild, from
' the narrower union. Only the item hash asks; the model reads the emitter as the
' `Shape` it is.
FUNC __canvas_particleEmitterItem(s AS Shape) AS DrawItem
  MATCH s
    CASE Rectangle(r)
      LET item AS DrawItem = r
      RETURN item
    CASE Line(l)
      LET item AS DrawItem = l
      RETURN item
    CASE Polygon(p)
      LET item AS DrawItem = p
      RETURN item
    CASE Circle(c)
      LET item AS DrawItem = c
      RETURN item
    CASE Arc(arc)
      LET item AS DrawItem = arc
      RETURN item
    CASE Text(tx)
      LET item AS DrawItem = tx
      RETURN item
    CASE Ellipse(e)
      LET item AS DrawItem = e
      RETURN item
    CASE PolyLine(pl)
      LET item AS DrawItem = pl
      RETURN item
    CASE PolyCurve(pc)
      LET item AS DrawItem = pc
      RETURN item
    CASE CurvePolygon(cp)
      LET item AS DrawItem = cp
      RETURN item
  END MATCH
  RETURN __canvas_noItem()
END FUNC

FUNC __canvas_particleHashColor(acc AS Integer, c AS color::Color) AS Integer
  RETURN __canvas_hashStep(acc, ((toInt(c.red) * 256 + toInt(c.green)) * 256 + toInt(c.blue)) * 256 + toInt(c.alpha))
END FUNC

' A `ParticleSystem`'s item hash: every field, the template's and the emitter's hashes
' and `time` included -- so a system presented again with a new `time` is a different
' scene to `present`, which is the whole of how one is animated. Never hashed natively:
' its `item` is a union field, which the structural walk does not fold
' (`func_item_hash.rs`).
FUNC __canvas_particleSystemHash(ps AS ParticleSystem) AS Integer
  MUT h AS Integer = __canvas_hashStart(14)
  h = __canvas_hashPoint(h, ps.position)
  h = __canvas_hashStep(h, __canvas_hashItem(__canvas_particleTemplate(ps.item)))
  h = __canvas_hashStep(h, __canvas_hashItem(__canvas_particleEmitterItem(ps.emitter)))
  LET e AS Emission = ps.emission
  MUT mode AS Integer = 0
  IF e.mode = EmissionMode.Burst THEN
    mode = 1
  END IF
  h = __canvas_hashStep(__canvas_hashStep(__canvas_hashStep(h, mode), e.maxParticles), e.burst)
  h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, e.rate), e.burstPeriod), e.lifetime), e.lifetimeVar)
  LET m AS Motion = ps.motion
  h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, m.speed), m.speedVar), m.angle), m.spread)
  h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, m.gravityX), m.gravityY), m.drag)
  h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, m.rotation), m.rotationVar), m.spin), m.spinVar)
  h = __canvas_hashFloat(__canvas_hashFloat(h, m.turbulence), m.turbulenceHz)
  LET a AS Appearance = ps.appearance
  h = __canvas_particleHashColor(__canvas_particleHashColor(h, a.startColor), a.endColor)
  h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, a.startScale), a.endScale), a.scaleVar)
  h = __canvas_hashStep(h, ps.seed)
  RETURN __canvas_hashFloat(h, ps.time)
END FUNC"#;

/// The hooks, for a program that constructs no `ParticleSystem`: it holds none, so none
/// of these is ever reached with one.
#[rustfmt::skip]
const ABSENT: &str =
r#"IMPORT canvas
IMPORT collections

FUNC __canvas_particleSystemHash(ps AS ParticleSystem) AS Integer
  RETURN 0
END FUNC"#;

/// The names the unconditional source calls: each is defined once in [`HOOKS_BODY`] and
/// once in [`ABSENT`], and exactly one of the two chunks is in any canvas program.
#[cfg(test)]
const HOOKS: [&str; 1] = ["__canvas_particleSystemHash"];

static BODY: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| format!(
        "IMPORT bits\nIMPORT canvas\nIMPORT collections\nIMPORT color\nIMPORT math\nIMPORT vector\n\n{MODEL}\n\n{EXPAND}\n\n{HOOKS_BODY}"
    ));

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "canvas_particles",
        gate: HelperGate::WhenUsedOrConstructed(&[], CONSTRUCTED),
        body: Some(BODY.as_str()),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "canvas_particles_absent",
        gate: HelperGate::WhenNotConstructed(CONSTRUCTED),
        body: Some(ABSENT),
        import_name: None,
        natively_called: false,
    });
}

#[cfg(test)]
mod tests;
