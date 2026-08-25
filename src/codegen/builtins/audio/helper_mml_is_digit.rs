//! `__audio_mmlIsDigit` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 E2: ASCII digit test as the range compare used by regex/datetime/json,
' replacing a 10-way OR chain. ch is a single scanned MML character.
FUNC __audio_mmlIsDigit(ch AS String) AS Boolean
  RETURN ch >= "0" AND ch <= "9"
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlIsDigit", BODY));
}
