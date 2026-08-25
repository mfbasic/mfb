//! `__audio_mmlClampFade` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Clamp a click-guard ramp to at most half the sounding length.
FUNC __audio_mmlClampFade(fade AS Integer, sound AS Integer) AS Integer
  MUT f AS Integer = fade
  IF f > sound / 2 THEN
    f = sound / 2
  END IF
  IF f < 0 THEN
    f = 0
  END IF
  RETURN f
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlClampFade", BODY));
}
