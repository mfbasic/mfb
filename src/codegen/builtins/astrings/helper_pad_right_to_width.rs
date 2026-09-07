//! `__astrings_padRightToWidth` — shared private helper for the `astrings` package.
//!
//! The Tier-B companion for `strings::padRightToWidth(AttributedString, …)`
//! (bug-528). Registered via `add_helper`; renders in the helper section of the
//! assembled source, in the order `mod.rs` calls the helpers. Body
//! byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_padRightToWidth(a AS AttributedString, columns AS Integer, padChar AS String = " ") AS AttributedString
  LET newText AS String = strings::padRightToWidth(toString(a), columns, padChar)
  RETURN __astrings_assemble(newText, astrings::readSpans(a))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_padRightToWidth", BODY));
}
