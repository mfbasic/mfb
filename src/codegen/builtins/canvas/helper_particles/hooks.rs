//! plan-176-R: the `helper_particles` MFBASIC helper text — the particle hooks (`HOOKS_BODY`).
//! Registration lives in `builtins/canvas/helper_particles/mod.rs`.

/// What a `ParticleSystem` is to the walks over a scene.
#[rustfmt::skip]
pub(super) const HOOKS_BODY: &str =
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
END FUNC

' A system's template as the item its particles are copies of: the shape, and for an
' `AutoPaint` the shape it wraps, with that shape's own paint. A particle is never under
' the mouse, so there is no state to show and no instance to note
' (`__canvas_autoAppend` is never reached from here).
FUNC __canvas_particleDrawn(t AS AutoShape) AS DrawItem
  MATCH t
    CASE AutoPaint(a)
      RETURN __canvas_autoItem(a)
    CASE ELSE
      RETURN __canvas_particleTemplate(t)
  END MATCH
END FUNC

' `__canvas_appendDraw`'s `ParticleSystem` arm: one draw entry per live particle.
'
' Each particle is appended as the ordinary shape it is -- its record built by the
' builders every item's is, and noted in `__CANVAS_DRAW_HASHES`, `_DX`, `_DY` and `_MAT`
' like any entry -- and everything that reads those lists (the render
' walk, the damage diff, both GPU predicates and emitters, `canvas::sceneLayout`, the
' `AutoPaint` resolve) needs no change, because none of them can tell. The enclosing
' groups' offset and matrix pass straight through: a system in a turned group is turned
' with it.
'
' The depth stop is SILENT, as `Group`'s is, and for the same reason: this runs on the
' graphics thread, where a raise has nowhere to go. A system's one raise is the
' worker's, in `__canvas_groupSignature`.
FUNC __canvas_particleAppend(offsets AS List OF Integer, ps AS ParticleSystem, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  MUT out AS List OF Integer = offsets
  IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN
    RETURN out
  END IF
  ' plan-150-B: where this system's entries start, for the damage diff -- which takes a
  ' system as one unit, and so has to be told of one with nothing alive too.
  LET first AS Integer = len(out)
  LET count AS Integer = __canvas_particleSlots(ps.emission)
  IF count <= 0 THEN
    __canvas_particleNote(first, 0)
    RETURN out
  END IF
  __canvas_particlePrepare(ps)
  LET template AS DrawItem = __canvas_particleDrawn(ps.item)
  LET look AS Appearance = ps.appearance
  MUT slot AS Integer = 0
  WHILE slot < count
    IF __canvas_particleState(ps, slot) THEN
      __canvas_particleTint(look, __CANVAS_PARTICLE_U)
      LET child AS DrawItem = __canvas_particleItem(template)
      IF __CANVAS_PARTICLE_DRAWN THEN
        LET hash AS Integer = __canvas_hashItem(child)
        LET offset AS Integer = __canvas_particleRecord(child)
        IF offset >= 0 THEN
          ' The entry `__canvas_appendDraw` would have made of it, less the cache.
          out = collections::append(out, offset)
          __canvas_noteEntry(offset, hash, gdx, gdy, mat)
        ELSE
          out = __canvas_appendDraw(out, child, hash, hash, gdx, gdy, mat, depth + 1, path, index)
        END IF
      END IF
    END IF
    slot = slot + 1
  END WHILE
  __canvas_particleNote(first, len(out) - first)
  RETURN out
END FUNC

' `__canvas_memoGroup`'s `ParticleSystem` arm: a system that is a group's child is one
' block per live particle in that group's run, and the answer is how many. A particle
' the cache holds is found there under the hash `__canvas_particleAppend` gave it; one
' that is not cached (`__canvas_particleRecord`) is built a second time for this walk.
FUNC __canvas_particleBlocks(ps AS ParticleSystem) AS Integer
  LET count AS Integer = __canvas_particleSlots(ps.emission)
  IF count <= 0 THEN
    RETURN 0
  END IF
  __canvas_particlePrepare(ps)
  LET template AS DrawItem = __canvas_particleDrawn(ps.item)
  LET look AS Appearance = ps.appearance
  MUT blocks AS Integer = 0
  MUT slot AS Integer = 0
  WHILE slot < count
    IF __canvas_particleState(ps, slot) THEN
      __canvas_particleTint(look, __CANVAS_PARTICLE_U)
      LET child AS DrawItem = __canvas_particleItem(template)
      IF __CANVAS_PARTICLE_DRAWN THEN
        MUT offset AS Integer = __canvas_particleRecord(child)
        IF offset < 0 THEN
          offset = __canvas_geometryFor(child, __canvas_hashItem(child))
        END IF
        __canvas_pushBlock(offset)
        blocks = blocks + 1
      END IF
    END IF
    slot = slot + 1
  END WHILE
  RETURN blocks
END FUNC

' Whether a shape names the font `handle`: a `Text` that lists it.
FUNC __canvas_particleFont(item AS DrawItem, handle AS Integer) AS Boolean
  MATCH item
    CASE Text(t)
      RETURN __canvas_textNamesFont(t, handle)
    CASE ELSE
      RETURN FALSE
  END MATCH
END FUNC

' Whether a system names a font: as its template (a `Text`, or an `AutoPaint` around
' one) or as its emitter. The group ownership rules count it (`__canvas_closeRetired`).
FUNC __canvas_particleNamesFont(ps AS ParticleSystem, handle AS Integer) AS Boolean
  IF handle = 0 THEN
    RETURN FALSE
  END IF
  IF __canvas_particleFont(__canvas_particleDrawn(ps.item), handle) THEN
    RETURN TRUE
  END IF
  RETURN __canvas_particleFont(__canvas_particleEmitterItem(ps.emitter), handle)
END FUNC

' Close the font of a `Text` in a retired group buffer's system, when nothing live
' names it -- the rule `__canvas_closeRetired` applies to a `Text` item.
FUNC __canvas_particleCloseText(item AS DrawItem, scene AS List OF DrawItem) AS Nothing
  MATCH item
    CASE Text(t)
      __canvas_closeTextFonts(t, scene)
    CASE ELSE
  END MATCH
END FUNC

FUNC __canvas_particleCloseFonts(ps AS ParticleSystem, scene AS List OF DrawItem) AS Nothing
  __canvas_particleCloseText(__canvas_particleDrawn(ps.item), scene)
  __canvas_particleCloseText(__canvas_particleEmitterItem(ps.emitter), scene)
END FUNC"#;
