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
    std::sync::LazyLock::new(|| format!("IMPORT canvas\nIMPORT collections\nIMPORT color\nIMPORT vector\n\n{HOOKS_BODY}"));

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
