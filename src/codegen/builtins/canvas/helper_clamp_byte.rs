//! `__canvas_clampByte` — the component clamp on the item-decode path.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Clamping rather than erroring is the deliberate choice: colour components are
/// routinely computed (`base + delta`, a lerp, a channel scaled by a fraction), and
/// a value that lands one past an end is a rounding artefact, not a program bug
/// worth trapping.
///
/// It was written as the shared clamp behind `canvas::rgb`/`rgba`, both of which
/// plan-122-D removed — the constructors and their clamping promise now live in
/// `color` (`__color_clampByte`). This copy stays because it has a second caller
/// that has nothing to do with either: `helper_items.rs`'s `__canvas_geoByte`
/// (`RETURN __canvas_clampByte(toInt(__canvas_geoAt(offset, slot)))`), which
/// narrows a `Float` read back out of the flattened geometry buffer. That is a
/// decode path, not a construction path, so routing it through `color` would make
/// the item decoder depend on the colour package for a two-comparison clamp.
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
