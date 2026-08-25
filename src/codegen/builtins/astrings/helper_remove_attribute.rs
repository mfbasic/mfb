//! `__astrings_removeAttribute` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_removeAttribute(a AS AttributedString, start AS Integer, endIndex AS Integer, attr AS Attribute) AS AttributedString
  LET n AS Integer = __astrings_validateRange(a, start, endIndex)
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  LET target AS AttrSpan = __astrings_encodeAttr(start, endIndex, 0, attr)
  MUT out AS List OF AttrSpan = []
  FOR EACH s IN spans
    IF __astrings_attrEquals(s, target) THEN
      out = __astrings_splitSpan(out, s, start, endIndex)
    ELSE
      out = collections::append(out, s)
    END IF
  NEXT
  RETURN astrings::writeSpans(a, out)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_removeAttribute", BODY));
}
