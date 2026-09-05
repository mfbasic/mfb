//! `__json_parseNumber` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseNumber(bytes AS List OF Byte, index AS Integer) AS __json_Node
  LET token AS __json_StringNode = __json_collectNumber(bytes, index)
  IF __json_validNumber(token.value) = FALSE THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  LET numberValue AS Float = __json_toNumber(token.value)
  LET value AS Json = JsonNum[numberValue]
  RETURN __json_Node[value, token.index]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseNumber", BODY));
}
