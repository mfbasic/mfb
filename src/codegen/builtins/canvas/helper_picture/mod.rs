//! `canvas::Picture` drawn inside a shape (plan-172-L).
//!
//! [`PICTURE_GEOMETRY`] is the arithmetic, as pure functions over numbers and point
//! lists — where an image goes inside a box under each `PictureFit`, the flattened
//! outline of a circle or an ellipse, and the intersection of two clips — so each is a
//! row of inputs and an expected output (`tests/canvas/rt_canvas_picture_fit.rs`, which
//! compiles this text and runs it).
//!
//! [`PICTURE`] is the item: what records a `Picture` draws from, how they are cached,
//! and what it is hit as. A picture is **one or two ordinary draw entries**, made where
//! the draw list is built —
//!
//! * a **plain** picture — an image stretched over a square-cornered rectangle — is one
//!   picture record, the rectangle's own re-kinded, exactly as it was before a picture
//!   had a shape;
//! * a **shaped** one is the image, a picture record over the fitted destination
//!   rectangle with the shape's outline for its clip, and then the outline, the shape
//!   itself with no fill —
//!
//! so the software renderer, both GPU backends, the damage diff and the group matrix
//! draw them with no change, and there is no new geometry kind, header slot or shader.
//!
//! Both chunks are gated on the program constructing a `Picture`
//! ([`HelperGate::WhenUsedOrConstructed`]): a canvas program with none compiles none of
//! it, and gets [`ABSENT`] for the names the unconditional source calls.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

mod geometry;
mod item;
use geometry::PICTURE_GEOMETRY;
use item::PICTURE;

/// The record types whose construction opens this chunk.
pub(super) const CONSTRUCTED: &[&str] = &["Picture", "Tile"];

/// What the unconditional source calls in a program that constructs no `Picture`
/// ([`HelperGate::WhenNotConstructed`]). Such a program holds none, so these cannot be
/// reached with one; they are here because `DrawItem` has the variant in every
/// program, and a `MATCH` over it must have its arm.
#[rustfmt::skip]
const ABSENT: &str =
r#"IMPORT canvas
IMPORT collections

FUNC __canvas_pictureAppend(offsets AS List OF Integer, pic AS Picture, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  RETURN offsets
END FUNC

FUNC __canvas_pictureBlocks(pic AS Picture, hash AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  RETURN []
END FUNC

FUNC __canvas_pictureAutoBlocks(host AS Integer, a AS AutoPaint, hash AS Integer, shown AS Integer) AS List OF Integer
  RETURN []
END FUNC

FUNC __canvas_pictureHash(pic AS Picture) AS Integer
  RETURN 0
END FUNC

FUNC __canvas_pictureHitItem(pic AS Picture) AS DrawItem
  RETURN __canvas_noItem()
END FUNC

FUNC __canvas_pictureShapeId(pic AS Picture) AS String
  RETURN ""
END FUNC

FUNC __canvas_pictureNamesFont(pic AS Picture, handle AS Integer) AS Boolean
  RETURN FALSE
END FUNC

FUNC __canvas_tileGeometry(tile AS Tile, hash AS Integer) AS Integer
  RETURN 0 - 1
END FUNC

FUNC __canvas_pictureCloseFont(pic AS Picture, scene AS List OF DrawItem) AS Nothing
END FUNC"#;

/// The names the unconditional source calls: each is defined once in [`PICTURE`] and
/// once in [`ABSENT`], and exactly one of the two chunks is in any canvas program.
#[cfg(test)]
const HOOKS: [&str; 9] = [
    "__canvas_pictureAppend",
    "__canvas_pictureBlocks",
    "__canvas_pictureAutoBlocks",
    "__canvas_pictureHash",
    "__canvas_pictureHitItem",
    "__canvas_pictureShapeId",
    "__canvas_pictureNamesFont",
    "__canvas_tileGeometry",
    "__canvas_pictureCloseFont",
];

static BODY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "IMPORT canvas\nIMPORT collections\nIMPORT math\nIMPORT vector\n\n{PICTURE_GEOMETRY}\n\n{PICTURE}"
    )
});

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "canvas_picture",
        gate: HelperGate::WhenUsedOrConstructed(&[], CONSTRUCTED),
        body: Some(BODY.as_str()),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "canvas_picture_absent",
        gate: HelperGate::WhenNotConstructed(CONSTRUCTED),
        body: Some(ABSENT),
        import_name: None,
        natively_called: false,
    });
}

#[cfg(test)]
mod tests;
