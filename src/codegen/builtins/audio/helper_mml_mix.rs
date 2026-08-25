//! `__audio_mmlMix` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Mix several sample tracks by summing (with clamping), padding shorter tracks
' with silence.
FUNC __audio_mmlMix(tracks AS List OF List OF Integer) AS List OF Integer
  MUT maxLen AS Integer = 0
  FOR EACH tr IN tracks
    IF len(tr) > maxLen THEN
      maxLen = len(tr)
    END IF
  NEXT
  MUT out AS List OF Integer = []
  FOR i = 0 TO maxLen - 1
    MUT acc AS Integer = 0
    FOR EACH tr IN tracks
      IF i < len(tr) THEN
        acc = acc + collections::get(tr, i)
      END IF
    NEXT
    out = collections::append(out, __audio_clampS16(acc))
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlMix", BODY));
}
