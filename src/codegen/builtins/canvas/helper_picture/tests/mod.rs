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
    // plan-175-A: a record is stored under the key its LOOKUP chose
    // (`__canvas_pictureCachedOf(g, …)` plans `g`; `__canvas_pictureStore` stores), so
    // the key a record lands under is the lookup's first argument. Under the item's own
    // geometry identity: the plain probe and the walk's cached-item probe
    // (`__canvas_imageCached`), which store only through the plain build...
    assert_eq!(
        PICTURE
            .matches("__canvas_pictureCachedOf(geo, hash, shadow)")
            .count(),
        2
    );
    assert_eq!(
        PICTURE
            .matches("__canvas_pictureStore(hash, __canvas_picturePlain(")
            .count(),
        1
    );
    // ...and the Tile's one record, under its own.
    let tile = PICTURE
        .split("FUNC __canvas_tileGeometry")
        .nth(1)
        .and_then(|s| s.split("END FUNC").next())
        .expect("Tile geometry helper");
    assert_eq!(
        tile.matches("__canvas_pictureCachedOf(g, d, shadow)")
            .count(),
        1
    );
    // Every other lookup is under a stepped key (`__canvas_pictureKey`, directly or as
    // `imageGeo`/`outlineGeo`), so a shaped picture's records cannot join them.
    for line in PICTURE
        .lines()
        .filter(|l| l.contains("= __canvas_pictureCachedOf("))
    {
        let stepped = [
            "(geo, hash, shadow)",
            "(g, d, shadow)",
            "(__canvas_pictureKey(",
            "(imageGeo,",
            "(outlineGeo,",
        ];
        assert!(stepped.iter().any(|s| line.contains(s)), "{line}");
    }
    for line in PICTURE
        .lines()
        .filter(|l| l.contains("__canvas_geometryOf("))
    {
        assert!(
            line.trim_end().ends_with(", outlineGeo, outlineKey)"),
            "{line}"
        );
    }
}
