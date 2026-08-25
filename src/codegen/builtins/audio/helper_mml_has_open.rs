//! `__audio_mmlHasOpen` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Whether any token is a `{` (used to detect an unclosed group after expansion).
FUNC __audio_mmlHasOpen(tokens AS List OF String) AS Boolean
  FOR EACH tk IN tokens
    IF tk = "{" THEN
      RETURN TRUE
    END IF
  NEXT
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlHasOpen", BODY));
}
