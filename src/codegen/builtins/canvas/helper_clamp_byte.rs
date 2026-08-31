//! `__canvas_clampByte` — the shared component clamp behind `canvas::rgb`/`rgba`.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Clamping rather than erroring is the deliberate choice: colour components are
/// routinely computed (`base + delta`, a lerp, a channel scaled by a fraction), and
/// a value that lands one past an end is a rounding artefact, not a program bug
/// worth trapping. `canvas::rgb` is documented as clamping, so this is the whole
/// implementation of that promise.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_clampByte(value AS Integer) AS Byte
  IF value < 0 THEN
    RETURN toByte(0)
  END IF
  IF value > 255 THEN
    RETURN toByte(255)
  END IF
  RETURN toByte(value)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_clampByte", BODY));
}
