//! `__astrings_padLeft` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_padLeft(a AS AttributedString, width AS Integer, padChar AS String = " ") AS AttributedString
  LET text AS String = toString(a)
  LET newText AS String = strings::padLeft(text, width, padChar)
  LET added AS Integer = __astrings_scalarCountStr(newText) - __astrings_scalarCountStr(text)
  RETURN __astrings_assemble(newText, __astrings_shiftSpans(astrings::readSpans(a), added))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_padLeft", BODY));
}
