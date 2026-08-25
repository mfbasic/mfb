//! `__astrings_right` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_right(a AS AttributedString, count AS Integer) AS AttributedString
  LET text AS String = toString(a)
  LET newText AS String = strings::right(text, count)
  LET n AS Integer = astrings::scalarLen(a)
  LET kept AS Integer = __astrings_scalarCountStr(newText)
  RETURN __astrings_assemble(newText, __astrings_windowSpans(astrings::readSpans(a), n - kept, n - 1))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_right", BODY));
}
