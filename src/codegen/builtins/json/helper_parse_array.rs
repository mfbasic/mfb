//! `__json_parseArray` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseArray(bytes AS List OF Byte, index AS Integer, depth AS Integer) AS __json_Node
  LET nextIndex AS Integer = __json_skipWhitespace(bytes, index)
  IF nextIndex >= len(bytes) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  IF toInt(collections::get(bytes, nextIndex)) = 93 THEN
    LET emptyItems AS List OF Json = []
    LET value AS Json = JsonArr[emptyItems]
    RETURN __json_Node[value, nextIndex + 1]
  END IF
  RETURN __json_parseArrayItems(bytes, nextIndex, [], depth)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseArray", BODY));
}
