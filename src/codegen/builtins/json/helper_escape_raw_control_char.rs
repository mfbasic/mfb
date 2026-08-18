//! `__json_escapeRawControlChar` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 D4: classify by the character's own scalar value instead of a linear
' probe over code points 0..31 (32 string constructions + 32 compares per call).
' encoding::utf32Encode yields ch's scalars in one call; a raw control char is a
' single scalar below 32.
FUNC __json_escapeRawControlChar(ch AS String) AS String
  LET cps AS List OF Integer = encoding::utf32Encode(ch)
  IF len(cps) = 1 THEN
    LET cp AS Integer = collections::get(cps, 0)
    IF cp < 32 THEN
      RETURN __json_controlEscape(cp)
    END IF
  END IF
  RETURN ch
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_escapeRawControlChar", BODY));
}
