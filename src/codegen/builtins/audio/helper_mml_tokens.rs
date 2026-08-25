//! `__audio_mmlTokens` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Split a track string into non-empty tokens, then expand its repeats.
FUNC __audio_mmlTokens(mml AS String) AS List OF String
  MUT tokens AS List OF String = []
  FOR EACH tk IN strings::split(mml, " ")
    IF len(tk) > 0 THEN
      tokens = collections::append(tokens, tk)
    END IF
  NEXT
  RETURN __audio_mmlExpand(tokens)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlTokens", BODY));
}
