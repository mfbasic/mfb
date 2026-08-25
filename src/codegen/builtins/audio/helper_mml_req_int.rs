//! `__audio_mmlReqInt` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Parse `T120`/`O4`/`L8`/`V7` etc.: the integer after the leading command char,
' validated to lo..hi. Raises on anything else.
FUNC __audio_mmlReqInt(token AS String, lo AS Integer, hi AS Integer, label AS String) AS Integer
  LET v AS Integer = __audio_mmlParseUint(strings::mid(token, 1, len(token) - 1))
  IF v < lo OR v > hi THEN
    FAIL error(77050002, "audio::play: " & label & " out of range in '" & token & "'")
  END IF
  RETURN v
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlReqInt", BODY));
}
