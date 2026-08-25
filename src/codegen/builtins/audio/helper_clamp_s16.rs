//! `__audio_clampS16` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 C7: shared s16 saturation, previously written three times (two as a pair
' of IFs, twice as an IF/ELSEIF — equivalent, since a value cannot exceed both
' bounds). Clamps to the signed 16-bit range.
FUNC __audio_clampS16(v AS Integer) AS Integer
  IF v > 32767 THEN
    RETURN 32767
  END IF
  IF v < -32768 THEN
    RETURN -32768
  END IF
  RETURN v
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_clampS16", BODY));
}
