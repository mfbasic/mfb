//! `__audio_mmlWaveCode` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The waveform code for an instrument name, or -1 if unknown.
FUNC __audio_mmlWaveCode(name AS String) AS Integer
  IF name = "sine" THEN
    RETURN 0
  ELSEIF name = "square" THEN
    RETURN 1
  ELSEIF name = "triangle" THEN
    RETURN 2
  ELSEIF name = "saw" THEN
    RETURN 3
  ELSEIF name = "noise" THEN
    RETURN 4
  END IF
  RETURN -1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlWaveCode", BODY));
}
