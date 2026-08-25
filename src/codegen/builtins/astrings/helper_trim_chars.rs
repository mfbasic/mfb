//! `__astrings_trimChars` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_trimChars(a AS AttributedString, chars AS String) AS AttributedString
  LET text AS String = toString(a)
  LET newText AS String = strings::trimChars(text, chars)
  LET leading AS Integer = __astrings_leadingInSet(text, chars)
  LET kept AS Integer = __astrings_scalarCountStr(newText)
  RETURN __astrings_assemble(newText, __astrings_windowSpans(astrings::readSpans(a), leading, leading + kept - 1))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_trimChars", BODY));
}
