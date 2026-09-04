//! `__color_clampFraction` — the shared `0.0`..`1.0` clamp for the perceptual
//! operations' `amount` parameter.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// `brighten`, `darken`, `mix`, `saturate` and `desaturate` all take an `amount`
/// fraction and all clamp it the same way, for the same reason the constructors
/// clamp their components: the value is usually computed, and one past an end is a
/// rounding artefact rather than a bug.
///
/// One helper rather than five inline `IF` pairs so the five members cannot drift
/// apart on the endpoint behaviour their exactness contracts depend on — a member
/// that clamped to `0.999` would make `brighten(c, 1.0)` land a step below white.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_clampFraction(amount AS Float) AS Float
  IF amount < 0.0 THEN
    RETURN 0.0
  END IF
  IF amount > 1.0 THEN
    RETURN 1.0
  END IF
  RETURN amount
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_clampFraction", BODY));
}
