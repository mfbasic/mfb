use super::*;

/// The signature line of `FUNC name(` in `source`.
fn signature<'a>(source: &'a str, name: &str) -> &'a str {
    let opens = format!("FUNC {name}(");
    let at = source
        .find(&opens)
        .unwrap_or_else(|| panic!("no `{opens}` in the chunk"));
    source[at..].lines().next().expect("a signature line")
}

/// The two chunks are alternatives: every hook is declared in both, with the same
/// parameters and the same return type, so the unconditional source compiles against
/// either. A hook added to one and not the other would build for one kind of program
/// and fail for the other.
#[test]
fn canvas_particle_hooks_are_declared_alike_in_both_chunks() {
    for hook in HOOKS {
        assert_eq!(
            signature(BODY.as_str(), hook),
            signature(ABSENT, hook),
            "{hook}"
        );
    }
    let declared = ABSENT.matches("\nFUNC ").count() + usize::from(ABSENT.starts_with("FUNC "));
    assert_eq!(
        declared,
        HOOKS.len(),
        "ABSENT declares a function that is not a hook"
    );
}

/// The model and the expansion run on the graphics thread, where a `FAIL` has no user
/// frame to unwind to: neither may raise. The raises a system can cause are the
/// worker's, in `present`.
#[test]
fn canvas_particle_model_never_raises() {
    for (name, source) in [("MODEL", MODEL), ("EXPAND", EXPAND)] {
        assert!(!source.contains("FAIL "), "{name} raises");
    }
}

/// The model is arithmetic over numbers and points: it names no record of the type set,
/// which is what lets `tests/canvas/rt_canvas_particles.rs` compile it as a program of
/// its own, without a canvas.
#[test]
fn canvas_particle_model_names_no_canvas_type() {
    for name in [
        "ParticleSystem",
        "Emission",
        "Motion",
        "Appearance",
        "Transform",
        "DrawItem",
        "AutoShape",
        "canvas::",
    ] {
        assert!(!MODEL.contains(name), "MODEL names `{name}`");
    }
}

/// The picture for a `(seed, time)` is the same on every target, so the model takes its
/// sine and cosine from the package (`helper_shapes.rs`) and writes its own
/// exponential; a platform's `libm` differs in the last place.
#[test]
fn canvas_particle_model_uses_no_platform_transcendental() {
    for source in [MODEL, EXPAND] {
        for name in ["math::sin", "math::cos", "math::exp", "math::pow", "math::tan"] {
            assert!(!source.contains(name), "the particle source calls `{name}`");
        }
    }
}

/// Every random draw a particle takes has a stream of its own, so changing one field's
/// variation cannot move another: 0 to 9 by name, and a polygon emitter's candidates
/// from 16 up.
#[test]
fn canvas_particle_draws_use_distinct_streams() {
    let spawn_at = EXPAND
        .find("FUNC __canvas_particleSpawn(")
        .expect("particleSpawn");
    let state_at = EXPAND
        .find("FUNC __canvas_particleState(")
        .expect("particleState");
    let spawn = &EXPAND[spawn_at..state_at];
    let state = &EXPAND[state_at..];
    for stream in [0, 3, 4, 5, 6, 7, 8, 9] {
        assert_eq!(
            state
                .matches(&format!("__canvas_particleHash(seed, slot, cycle, {stream})"))
                .count(),
            1,
            "stream {stream}"
        );
    }
    assert!(spawn.contains("__canvas_particleHash(seed, slot, cycle, 1)"));
    assert!(spawn.contains("__canvas_particleHash(seed, slot, cycle, 2)"));
    assert!(MODEL.contains("16 + k * 2") && MODEL.contains("17 + k * 2"));
}
