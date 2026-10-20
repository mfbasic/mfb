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

mod damage;
mod expand;
mod hooks;
mod model;
use damage::DAMAGE;
use expand::{DRAW, EXPAND};
use hooks::HOOKS_BODY;
use model::MODEL;

/// The record type whose construction opens this chunk.
pub(super) const CONSTRUCTED: &[&str] = &["ParticleSystem"];

/// The hooks, for a program that constructs no `ParticleSystem`: it holds none, so none
/// of these is ever reached with one.
#[rustfmt::skip]
const ABSENT: &str =
r#"IMPORT canvas
IMPORT collections

FUNC __canvas_particleSystemHash(ps AS ParticleSystem) AS Integer
  RETURN 0
END FUNC

FUNC __canvas_particleAppend(offsets AS List OF Integer, ps AS ParticleSystem, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  RETURN offsets
END FUNC

FUNC __canvas_particleBlocks(ps AS ParticleSystem) AS Integer
  RETURN 0
END FUNC

FUNC __canvas_particleNamesFont(ps AS ParticleSystem, handle AS Integer) AS Boolean
  RETURN FALSE
END FUNC

FUNC __canvas_particleCloseFonts(ps AS ParticleSystem, scene AS List OF DrawItem) AS Nothing
END FUNC

FUNC __canvas_particleBegin() AS Nothing
END FUNC

FUNC __canvas_particleOwned(index AS Integer, before AS Integer) AS Nothing
END FUNC

FUNC __canvas_particleLayout(spans AS List OF Integer) AS Nothing
END FUNC

FUNC __canvas_particleFramed() AS Boolean
  RETURN FALSE
END FUNC

FUNC __canvas_particleRemember(hashes AS List OF Integer, offsets AS List OF Integer) AS Nothing
END FUNC

FUNC __canvas_particleDamage(hashes AS List OF Integer, offsets AS List OF Integer, width AS Integer, height AS Integer) AS List OF Integer
  RETURN __canvas_damageFull(width, height)
END FUNC"#;

/// The names the unconditional source calls: each is defined once in [`HOOKS_BODY`] and
/// once in [`ABSENT`], and exactly one of the two chunks is in any canvas program.
#[cfg(test)]
const HOOKS: [&str; 11] = [
    "__canvas_particleSystemHash",
    "__canvas_particleAppend",
    "__canvas_particleBlocks",
    "__canvas_particleNamesFont",
    "__canvas_particleCloseFonts",
    "__canvas_particleBegin",
    "__canvas_particleOwned",
    "__canvas_particleLayout",
    "__canvas_particleFramed",
    "__canvas_particleRemember",
    "__canvas_particleDamage",
];

static BODY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "IMPORT bits\nIMPORT canvas\nIMPORT collections\nIMPORT color\nIMPORT math\nIMPORT vector\n\n{MODEL}\n\n{EXPAND}\n\n{DRAW}\n\n{DAMAGE}\n\n{HOOKS_BODY}"
    )
});

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
