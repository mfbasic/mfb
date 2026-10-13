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

' A `ParticleSystem`'s item hash: every field, the template's hash included. Never
' hashed natively -- its `item` is a union field, which the structural walk does not
' fold (`func_item_hash.rs`).
FUNC __canvas_particleSystemHash(ps AS ParticleSystem) AS Integer
  MUT h AS Integer = __canvas_hashStart(14)
  RETURN __canvas_hashStep(h, __canvas_hashItem(__canvas_particleTemplate(ps.item)))
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
    std::sync::LazyLock::new(|| format!("IMPORT canvas\nIMPORT collections\n\n{HOOKS_BODY}"));

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
