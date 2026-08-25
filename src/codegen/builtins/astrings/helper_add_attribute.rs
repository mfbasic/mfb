//! `__astrings_addAttribute` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_addAttribute(a AS AttributedString, start AS Integer, endIndex AS Integer, attr AS Attribute) AS AttributedString
  LET n AS Integer = __astrings_validateRange(a, start, endIndex)
  MUT spans AS List OF AttrSpan = astrings::readSpans(a)
  LET seq AS Integer = __astrings_nextSeq(spans)
  LET span AS AttrSpan = __astrings_encodeAttr(start, endIndex, seq, attr)
  spans = collections::append(spans, span)
  RETURN astrings::writeSpans(a, spans)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_addAttribute", BODY));
}
