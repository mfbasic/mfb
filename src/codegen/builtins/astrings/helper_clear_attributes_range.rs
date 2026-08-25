//! `__astrings_clearAttributesRange` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_clearAttributesRange(a AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString
  LET n AS Integer = __astrings_validateRange(a, start, endIndex)
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  MUT out AS List OF AttrSpan = []
  FOR EACH s IN spans
    out = __astrings_splitSpan(out, s, start, endIndex)
  NEXT
  RETURN astrings::writeSpans(a, out)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "astrings_clearAttributesRange",
        BODY,
    ));
}
