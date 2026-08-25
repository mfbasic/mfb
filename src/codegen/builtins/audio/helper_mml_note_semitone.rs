//! `__audio_mmlNoteSemitone` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' The chromatic offset of a note letter within an octave (C = 0). -1 if not A..G.
FUNC __audio_mmlNoteSemitone(letter AS String) AS Integer
  IF letter = "C" THEN
    RETURN 0
  ELSEIF letter = "D" THEN
    RETURN 2
  ELSEIF letter = "E" THEN
    RETURN 4
  ELSEIF letter = "F" THEN
    RETURN 5
  ELSEIF letter = "G" THEN
    RETURN 7
  ELSEIF letter = "A" THEN
    RETURN 9
  ELSEIF letter = "B" THEN
    RETURN 11
  END IF
  RETURN -1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlNoteSemitone", BODY));
}
