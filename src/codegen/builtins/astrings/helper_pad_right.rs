//! `__astrings_padRight` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_padRight(a AS AttributedString, width AS Integer, padChar AS String = " ") AS AttributedString
  LET newText AS String = strings::padRight(toString(a), width, padChar)
  RETURN __astrings_assemble(newText, astrings::readSpans(a))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_padRight", BODY));
}
