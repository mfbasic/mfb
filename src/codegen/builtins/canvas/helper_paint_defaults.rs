//! The zero values the `Paint` constructors fill in for the fields a caller did
//! not name.
//!
//! These exist because **MFBASIC named construction does not default unset
//! fields** — `Paint[fill := c]` is a `TYPE_CONSTRUCTOR_ARITY_MISMATCH`, not a
//! partially-specified record. So `Paint`'s "every field's zero value is that
//! field's no-op" rule is delivered by `canvas::fill`/`stroke`/`fillStroke`
//! writing those zeros explicitly, rather than by the constructor syntax.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Fully transparent — the no-op `Color`, and what an unnamed `Paint` channel is.
#[rustfmt::skip]
const TRANSPARENT: &str =
r#"FUNC __canvas_transparent() AS Color
  RETURN Color[red := toByte(0), green := toByte(0), blue := toByte(0), alpha := toByte(0)]
END FUNC"#;

/// The all-zero `Transform`, which `canvas` defines to mean the identity — see the
/// `Transform` type description. Writing the literal identity matrix here instead
/// would be wrong: it would make an explicitly-zero transform and a defaulted one
/// behave differently under a later `WITH`.
#[rustfmt::skip]
const NO_TRANSFORM: &str =
r#"FUNC __canvas_noTransform() AS Transform
  RETURN Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0]
END FUNC"#;

/// A zero-area `Bounds`, which means "no clip".
#[rustfmt::skip]
const NO_CLIP: &str =
r#"FUNC __canvas_noClip() AS Bounds
  RETURN Bounds[x := 0.0, y := 0.0, w := 0.0, h := 0.0]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_transparent", TRANSPARENT));
    pkg.add_helper(RegistryHelper::always("canvas_noTransform", NO_TRANSFORM));
    pkg.add_helper(RegistryHelper::always("canvas_noClip", NO_CLIP));
}
