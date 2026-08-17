//! `__json_isRawControlChar` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 D4: a raw control char is a single Unicode scalar below 32. Ask for
' ch's scalars directly (encoding::utf32Encode) rather than the old 32-step probe
' that rebuilt and compared a string for every code point 0..31.
FUNC __json_isRawControlChar(ch AS String) AS Boolean
  LET cps AS List OF Integer = encoding::utf32Encode(ch)
  IF len(cps) = 1 THEN
    RETURN collections::get(cps, 0) < 32
  END IF
  RETURN FALSE
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_isRawControlChar", BODY));
}
