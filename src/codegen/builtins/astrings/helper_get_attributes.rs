//! `__astrings_getAttributes` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_getAttributes(a AS AttributedString, index AS Integer) AS List OF Attribute
  LET n AS Integer = astrings::scalarLen(a)
  IF index < 0 OR index >= n THEN
    FAIL error(77050001, "attribute index out of bounds")
  END IF
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  MUT covering AS List OF AttrSpan = []
  FOR EACH s IN spans
    IF s.start <= index AND index <= s.last THEN
      covering = collections::append(covering, s)
    END IF
  NEXT
  MUT result AS List OF Attribute = []
  FOR EACH s IN covering
    IF __astrings_isWinner(s, covering) THEN
      result = collections::append(result, __astrings_decodeAttr(s))
    END IF
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_getAttributes", BODY));
}
