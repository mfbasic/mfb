//! `__audio_mmlFrames` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Frames for a note of the given length (1..64) at tempo, with `dots` dots. A
' quarter note (length 4) is one beat; one dot is x1.5, two dots x1.75, etc.
FUNC __audio_mmlFrames(tempo AS Integer, lengthN AS Integer, dots AS Integer) AS Integer
  LET wholeSeconds AS Float = (60.0 / toFloat(tempo)) * 4.0
  LET base AS Float = wholeSeconds / toFloat(lengthN)
  LET dotFactor AS Float = 2.0 - math::pow(0.5, toFloat(dots))
  RETURN toInt(base * dotFactor * 48000.0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlFrames", BODY));
}
