//! `__color_clampByte` — the shared component clamp behind every `color`
//! constructor.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Clamping rather than erroring is the deliberate choice: colour components are
/// routinely computed (`base + delta`, a lerp, a channel scaled by a fraction), and
/// a value that lands one past an end is a rounding artefact, not a program bug
/// worth trapping. Every `color` constructor is documented as clamping, so this is
/// the whole implementation of that promise.
///
/// The body is `canvas`'s `__canvas_clampByte`
/// (`crate::codegen::builtins::canvas::helper_clamp_byte`) under the new name —
/// deliberately the same expression, so plan-122-D's canvas rename cannot change a
/// clamped value.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_clampByte(value AS Integer) AS Byte
  IF value < 0 THEN
    RETURN toByte(0)
  END IF
  IF value > 255 THEN
    RETURN toByte(255)
  END IF
  RETURN toByte(value)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_clampByte", BODY));
}
