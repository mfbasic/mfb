//! `__astrings_padLeftToWidth` — shared private helper for the `astrings` package.
//!
//! The Tier-B companion for `strings::padLeftToWidth(AttributedString, …)`
//! (bug-528). Registered via `add_helper`; renders in the helper section of the
//! assembled source, in the order `mod.rs` calls the helpers. Body
//! byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_padLeftToWidth(a AS AttributedString, columns AS Integer, padChar AS String = " ") AS AttributedString
  LET text AS String = toString(a)
  LET newText AS String = strings::padLeftToWidth(text, columns, padChar)
  LET added AS Integer = __astrings_scalarCountStr(newText) - __astrings_scalarCountStr(text)
  RETURN __astrings_assemble(newText, __astrings_shiftSpans(astrings::readSpans(a), added))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_padLeftToWidth", BODY));
}
