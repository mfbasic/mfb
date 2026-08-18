//! `__json_parseArray` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseArray(chars AS List OF String, index AS Integer, depth AS Integer) AS __json_Node
  LET nextIndex AS Integer = __json_skipWhitespace(chars, index)
  IF nextIndex >= len(chars) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  IF collections::get(chars, nextIndex) = "]" THEN
    LET emptyItems AS List OF Json = []
    LET value AS Json = JsonArr[emptyItems]
    RETURN __json_Node[value, nextIndex + 1]
  END IF
  RETURN __json_parseArrayItems(chars, nextIndex, [], depth)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseArray", BODY));
}
