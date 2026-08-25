//! `__astrings_encodeAttr` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_encodeAttr(start AS Integer, endIndex AS Integer, seq AS Integer, attr AS Attribute) AS AttrSpan
  MATCH attr
    CASE AttrFlag(f)
      RETURN AttrSpan[start, endIndex, seq, 0, __astrings_flagMember(f.kind), "", 0]
    CASE AttrText(t)
      RETURN AttrSpan[start, endIndex, seq, 1, 0, t.value, 0]
    CASE AttrNumber(n)
      RETURN AttrSpan[start, endIndex, seq, 2, __astrings_numberMember(n.kind), "", n.value]
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_encodeAttr", BODY));
}
