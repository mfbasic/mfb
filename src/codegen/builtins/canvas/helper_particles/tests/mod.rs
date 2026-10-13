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
