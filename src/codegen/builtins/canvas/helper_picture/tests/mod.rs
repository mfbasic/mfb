//! `codegen/builtins/canvas/helper_picture/mod.rs`'s unit tests, moved out of the production file so the coverage
//! denominator measures only production lines (planning/tests.md C15).

use super::{ABSENT, HOOKS, PICTURE};

fn signature<'a>(source: &'a str, name: &str) -> &'a str {
    let opens = format!("FUNC {name}(");
    let at = source
        .find(&opens)
        .unwrap_or_else(|| panic!("no `{opens}` in the chunk"));
    source[at..].lines().next().expect("a signature line")
}

/// The two chunks are alternatives (as `helper_autopaint.rs`'s are): every hook is
/// declared in both with one signature, so the unconditional source compiles
/// against either.
#[test]
fn canvas_picture_hooks_are_declared_alike_in_both_chunks() {
    for hook in HOOKS {
        assert_eq!(signature(PICTURE, hook), signature(ABSENT, hook), "{hook}");
    }
    let declared = ABSENT.matches("\nFUNC ").count() + usize::from(ABSENT.starts_with("FUNC "));
    assert_eq!(
        declared,
        HOOKS.len(),
        "ABSENT declares a function that is not a hook"
    );
}

/// A shaped picture's records are never cached under the item's own hash: the
/// native first pass would then resolve the item to ONE record and the walk that
/// makes its two entries would never see it. Only the plain path inserts under
/// `hash`. A Tile is also one record and deliberately uses its own item hash; the
/// exact count remains pinned so a shaped-picture path cannot join them.
#[test]
fn canvas_picture_only_the_plain_record_is_cached_under_the_item_hash() {
    assert_eq!(PICTURE.matches("__canvas_pictureInsert(hash,").count(), 2);
    let tile = PICTURE
        .split("FUNC __canvas_tileGeometry")
        .nth(1)
        .and_then(|s| s.split("END FUNC").next())
        .expect("Tile geometry helper");
    assert_eq!(tile.matches("__canvas_pictureInsert(hash,").count(), 1);
    for line in PICTURE
        .lines()
        .filter(|l| l.contains("__canvas_geometryFor("))
    {
        assert!(line.trim_end().ends_with(", outlineKey)"), "{line}");
    }
}
