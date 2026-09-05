//! `__json_parseNumber` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-510 (DEC-03): find the token's end, validate the grammar over its bytes,
' and only then decode it -- once -- for toFloat. The old order (slice to a
' String, graphemize it to validate) cost ~1.5 KB per number.
FUNC __json_parseNumber(bytes AS List OF Byte, index AS Integer) AS __json_Node
  LET endIndex AS Integer = __json_numberEnd(bytes, index)
  IF __json_validNumber(bytes, index, endIndex) = FALSE THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  LET token AS String = encoding::utf8Decode(collections::mid(bytes, index, endIndex - index))
  LET numberValue AS Float = __json_toNumber(token)
  LET value AS Json = JsonNum[numberValue]
  RETURN __json_Node[value, endIndex]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseNumber", BODY));
}
