//! `__astrings_decodeAttr` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_decodeAttr(s AS AttrSpan) AS Attribute
  IF s.class = 0 THEN
    RETURN AttrFlag[__astrings_flagFromMember(s.member)]
  END IF
  IF s.class = 1 THEN
    RETURN AttrText[AttrTypeText.Font, s.text]
  END IF
  RETURN AttrNumber[__astrings_numberFromMember(s.member), s.number]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_decodeAttr", BODY));
}
