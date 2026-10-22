//! `codegen/builtins/canvas/helper_autopaint.rs`'s unit tests, moved out of the production file so the coverage
//! denominator measures only production lines (planning/tests.md C15).

use super::{ABSENT, HOOKS, RESOLVE};

/// The signature line of `FUNC name(` in `source`.
fn signature<'a>(source: &'a str, name: &str) -> &'a str {
    let opens = format!("FUNC {name}(");
    let at = source
        .find(&opens)
        .unwrap_or_else(|| panic!("no `{opens}` in the chunk"));
    source[at..].lines().next().expect("a signature line")
}

/// The two chunks are alternatives: every hook is declared in both, with the same
/// parameters and the same return type, so the unconditional source compiles
/// against either. A hook added to one and not the other would build for one kind
/// of program and fail for the other.
#[test]
fn canvas_auto_hooks_are_declared_alike_in_both_chunks() {
    for hook in HOOKS {
        assert_eq!(signature(RESOLVE, hook), signature(ABSENT, hook), "{hook}");
    }
    let declared = ABSENT.matches("\nFUNC ").count() + usize::from(ABSENT.starts_with("FUNC "));
    assert_eq!(
        declared,
        HOOKS.len(),
        "ABSENT declares a function that is not a hook"
    );
}

/// The hit test reads the entries the walk appended, before any is swapped for a
/// state paint's record — the rule that a state paint never changes where a shape
/// is hit. In the source that is an order: the one call to `__canvas_autoHitAt`
/// comes before the first `collections::set(out, …)`.
#[test]
fn canvas_auto_the_hit_test_runs_before_any_entry_is_swapped() {
    let resolve = &RESOLVE[RESOLVE.find("FUNC __canvas_autoResolve(").expect("resolve")..];
    let hit = resolve
        .find("over = __canvas_autoHitAt(")
        .expect("the hit test");
    let swap = resolve
        .find("out = collections::set(out,")
        .expect("the swap");
    assert!(hit < swap);
}
