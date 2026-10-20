//! plan-176-R: the `helper_particles` MFBASIC helper text — expanding and drawing particles
//! (`EXPAND`, `DRAW`). Registration lives in `builtins/canvas/helper_particles/mod.rs`.

/// What reads a `ParticleSystem` into [`MODEL`]: the emitter prepared once per system
/// per frame, and one particle's state from its slot.
///
/// **The emitter is prepared, not re-read per particle.** A `PolyCurve` is flattened, a
/// `Text` is shaped and measured, a `Polygon`'s rings are copied out of the item — once,
/// into the globals below — and every slot of the system then samples from those. They
/// are graphics-thread globals, like everything the frame's walk keeps
/// (`.ai/canvas-threading.md` section 2).
#[rustfmt::skip]
pub(super) const EXPAND: &str =
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
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text, t.letterSpacing)
  IF len(shaped) < __CANVAS_RUN_STRIDE OR NOT __canvas_runDrawable(shaped, scales) THEN
    RETURN
  END IF
  LET advance AS Float = __canvas_runPen(shaped, scales, 0.0, t.letterSpacing)
  LET line AS List OF Float = __canvas_runLine(handles, scales, shaped)
  LET up AS Float = collections::getOr(line, 0, 0.0)
  LET down AS Float = collections::getOr(line, 1, 0.0)
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

/// The expansion (plan-150-A section 6): a system's live particles as ordinary items.
///
/// **A particle is its template drawn as if it sat alone in a group node at the
/// particle's point, turned and scaled there.** `__canvas_particleItem` flattens that
/// node into the shape's own `position` and `transform` and multiplies the tint into its
/// paint, so what the walk appends is one ordinary shape — one geometry record, one
/// item block — and nothing downstream of the walk knows a particle from any other
/// item. No shader, pipeline, binding or buffer layout is involved.
#[rustfmt::skip]
pub(super) const DRAW: &str =
r#"' The tint of the particle being built: each channel 0 to 255, multiplied into the
' template's fill and stroke.
MUT __CANVAS_PARTICLE_TINT_R AS Integer = 255
MUT __CANVAS_PARTICLE_TINT_G AS Integer = 255
MUT __CANVAS_PARTICLE_TINT_B AS Integer = 255
MUT __CANVAS_PARTICLE_TINT_A AS Integer = 255

' One colour channel `u` of the way from `s` to `e`, in LINEAR light -- the space the
' rasteriser and both shaders composite in. Interpolating the stored sRGB values would
' pass a fade through a midpoint that is visibly too dark.
FUNC __canvas_particleChannel(s AS Byte, e AS Byte, u AS Float) AS Integer
  LET lo AS Float = toFloat(color::toLinear(s))
  LET hi AS Float = toFloat(color::toLinear(e))
  RETURN toInt(color::fromLinear(toInt(lo + (hi - lo) * u + 0.5)))
END FUNC

' The tint at `u` of the way through a particle's life. Alpha is not a light level and
' is interpolated as it is stored.
FUNC __canvas_particleTint(look AS Appearance, u AS Float) AS Nothing
  LET s AS color::Color = look.startColor
  LET e AS color::Color = look.endColor
  __CANVAS_PARTICLE_TINT_R = __canvas_particleChannel(s.red, e.red, u)
  __CANVAS_PARTICLE_TINT_G = __canvas_particleChannel(s.green, e.green, u)
  __CANVAS_PARTICLE_TINT_B = __canvas_particleChannel(s.blue, e.blue, u)
  LET lo AS Float = toFloat(toInt(s.alpha))
  __CANVAS_PARTICLE_TINT_A = toInt(lo + (toFloat(toInt(e.alpha)) - lo) * u + 0.5)
END FUNC

' A colour under the tint: each channel multiplied, alpha included, so white leaves it
' as it was and a transparent tint hides it. The rule a `Picture`'s fill follows.
FUNC __canvas_particleTinted(c AS color::Color) AS color::Color
  RETURN color::rgba((toInt(c.red) * __CANVAS_PARTICLE_TINT_R + 127) / 255, (toInt(c.green) * __CANVAS_PARTICLE_TINT_G + 127) / 255, (toInt(c.blue) * __CANVAS_PARTICLE_TINT_B + 127) / 255, (toInt(c.alpha) * __CANVAS_PARTICLE_TINT_A + 127) / 255)
END FUNC

' A paint under the tint: its fill and its stroke. A gradient's stops are not reached.
FUNC __canvas_particlePaint(p AS Paint) AS Paint
  IF __CANVAS_PARTICLE_TINT_R = 255 AND __CANVAS_PARTICLE_TINT_G = 255 AND __CANVAS_PARTICLE_TINT_B = 255 AND __CANVAS_PARTICLE_TINT_A = 255 THEN
    RETURN p
  END IF
  RETURN WITH p { fill := __canvas_particleTinted(p.fill), stroke := __canvas_particleTinted(p.stroke) }
END FUNC

' `__canvas_particleCompose`'s answers: the position and transform of the shape that IS
' the particle, and whether there is one.
MUT __CANVAS_PARTICLE_AT AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
MUT __CANVAS_PARTICLE_T AS Transform = Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0]
MUT __CANVAS_PARTICLE_DRAWN AS Boolean = FALSE

' The particle's node -- at the particle's point P, with M = turn(ROT) . scale(SIZE) --
' flattened into a template shape's own fields. With the template's transform A | t (the
' identity and nothing when it is not in effect) and its position q, a local point p
' reaches the surface at
'
'     P + M(q + A p + t)  =  P + (M A) p + M(q + t)
'
' so the particle is the same shape at `position := P` with
' `transform := [ M A | M(q + t) ]`. The template's own `position` and `transform` are
' therefore offsets INSIDE the particle, turned and scaled with it.
'
' A particle that is neither turned nor scaled needs none of it: it is the template
' moved, `position := P + q`, with its transform as it was -- which keeps an unturned
' particle on the path every untransformed item takes.
'
' FALSE when M A has no area -- a scale that reached zero. That particle is not drawn:
' the shape rule treats a transform with no area as the identity, so it would otherwise
' be drawn at FULL size at the moment it was meant to vanish.
FUNC __canvas_particleCompose(q AS vector::Float2, t AS Transform) AS Boolean
  LET size AS Float = __CANVAS_PARTICLE_SIZE
  IF __CANVAS_PARTICLE_ROT = 0.0 AND size = 1.0 THEN
    __CANVAS_PARTICLE_AT = vector::Float2[x := __CANVAS_PARTICLE_PX + q.x, y := __CANVAS_PARTICLE_PY + q.y]
    __CANVAS_PARTICLE_T = t
    RETURN TRUE
  END IF
  __canvas_particleUnit(__CANVAS_PARTICLE_ROT)
  LET ma AS Float = size * __CANVAS_PARTICLE_UX
  LET mb AS Float = size * __CANVAS_PARTICLE_UY
  LET mc AS Float = 0.0 - mb
  LET md AS Float = ma
  MUT aa AS Float = 1.0
  MUT ab AS Float = 0.0
  MUT ac AS Float = 0.0
  MUT ad AS Float = 1.0
  MUT ox AS Float = q.x
  MUT oy AS Float = q.y
  IF __canvas_transformActive(t) THEN
    aa = t.a
    ab = t.b
    ac = t.c
    ad = t.d
    ox = q.x + t.tx
    oy = q.y + t.ty
  END IF
  LET na AS Float = ma * aa + mc * ab
  LET nb AS Float = mb * aa + md * ab
  LET nc AS Float = ma * ac + mc * ad
  LET nd AS Float = mb * ac + md * ad
  IF NOT (__canvas_absF(na * nd - nb * nc) >= 0.000000000001) THEN
    RETURN FALSE
  END IF
  __CANVAS_PARTICLE_AT = vector::Float2[x := __CANVAS_PARTICLE_PX, y := __CANVAS_PARTICLE_PY]
  __CANVAS_PARTICLE_T = Transform[a := na, b := nb, c := nc, d := nd, tx := ma * ox + mc * oy, ty := mb * ox + md * oy]
  RETURN TRUE
END FUNC

' A `PolyLine`'s per-point colours under the tint: they are what it is stroked with.
FUNC __canvas_particleColors(colors AS List OF color::Color) AS List OF color::Color
  IF len(colors) = 0 OR (__CANVAS_PARTICLE_TINT_R = 255 AND __CANVAS_PARTICLE_TINT_G = 255 AND __CANVAS_PARTICLE_TINT_B = 255 AND __CANVAS_PARTICLE_TINT_A = 255) THEN
    RETURN colors
  END IF
  MUT out AS List OF color::Color = []
  FOR EACH c IN colors
    out = collections::append(out, __canvas_particleTinted(c))
  NEXT
  RETURN out
END FUNC

' The item one particle is: the template, placed, turned, scaled and tinted from the
' state `__canvas_particleState` and `__canvas_particleTint` left. `__CANVAS_PARTICLE_DRAWN`
' says whether there is one; when there is not, the answer is the item that draws
' nothing.
FUNC __canvas_particleItem(template AS DrawItem) AS DrawItem
  __CANVAS_PARTICLE_DRAWN = FALSE
  MATCH template
    CASE Rectangle(r)
      IF __canvas_particleCompose(r.position, r.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS Rectangle = WITH r { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(r.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Line(l)
      IF __canvas_particleCompose(l.position, l.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS Line = WITH l { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(l.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Polygon(p)
      IF __canvas_particleCompose(p.position, p.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS Polygon = WITH p { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(p.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Circle(c)
      IF __canvas_particleCompose(c.position, c.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS Circle = WITH c { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(c.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Arc(arc)
      IF __canvas_particleCompose(arc.position, arc.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS Arc = WITH arc { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(arc.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Text(tx)
      IF __canvas_particleCompose(tx.position, tx.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS Text = WITH tx { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(tx.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Ellipse(e)
      IF __canvas_particleCompose(e.position, e.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS Ellipse = WITH e { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(e.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE PolyLine(pl)
      IF __canvas_particleCompose(pl.position, pl.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS PolyLine = WITH pl { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(pl.paint), colors := __canvas_particleColors(pl.colors) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE PolyCurve(pc)
      IF __canvas_particleCompose(pc.position, pc.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS PolyCurve = WITH pc { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(pc.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE CurvePolygon(cp)
      IF __canvas_particleCompose(cp.position, cp.transform) THEN
        __CANVAS_PARTICLE_DRAWN = TRUE
        LET changed AS CurvePolygon = WITH cp { position := __CANVAS_PARTICLE_AT, transform := __CANVAS_PARTICLE_T, paint := __canvas_particlePaint(cp.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE ELSE
  END MATCH
  RETURN __canvas_noItem()
END FUNC

' plan-150-B: a particle's geometry record, built and NOT cached -- or -1 for a particle
' the native builder does not take, which goes the way every item does.
'
' The geometry cache earns nothing from a particle. Every live particle has a hash of
' its own (its own position, its own tint) and a new one next frame, so each was a
' lookup that missed, a slot, a table entry, and a slot dropped at the next frame
' boundary -- for a record read once. So the record the native builder makes
' (`canvas::geoBuild`: a shape with no transform in effect and no gradient, which is
' every particle that is neither turned nor scaled) is appended straight to
' `__CANVAS_GEO_DATA` and its offset used, with no slot and no index entry.
'
' What keeps that offset good for the frame is the rule that keeps every offset good:
' nothing moves or drops a float of `__CANVAS_GEO_DATA` inside a frame; the one place
' that does is `canvas::geoBeginFrame`, at the top of the next. There, floats no slot
' used count as stale, and the pass that keeps the used slots drops them with the rest
' (`.ai/canvas-threading.md` section 14).
FUNC __canvas_particleRecord(child AS DrawItem) AS Integer
  LET native AS List OF Float = canvas::geoBuild(child)
  IF len(native) = 0 THEN
    RETURN 0 - 1
  END IF
  __canvas_geoVerify(child, native)
  __CANVAS_GEO_NATIVE_BUILDS = __CANVAS_GEO_NATIVE_BUILDS + 1
  LET offset AS Integer = len(__CANVAS_GEO_DATA)
  __CANVAS_GEO_DATA = collections::append(__CANVAS_GEO_DATA, native)
  RETURN offset
END FUNC

' How many slots of a system can hold a particle: the pool, or in `Burst` mode the
' burst when that is smaller.
FUNC __canvas_particleSlots(e AS Emission) AS Integer
  IF e.mode = EmissionMode.Burst AND e.burst < e.maxParticles THEN
    RETURN e.burst
  END IF
  RETURN e.maxParticles
END FUNC"#;
