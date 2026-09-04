//! `__json_parseObjectItems` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseObjectItems(bytes AS List OF Byte, index AS Integer, fields AS Map OF String TO Json, depth AS Integer) AS __json_Node
  ' Iterative accumulation, mirroring the array parser. `acc = set(acc, key, val)`
  ' uses the in-place MUT map set once Phase 3 lands; today it is the rebuild path
  ' (no regression). Eliminating the recursion also avoids deep call stacks on
  ' large objects (plan-02 Phase 4 fallback).
  MUT acc AS Map OF String TO Json = fields
  MUT idx AS Integer = index
  MUT finished AS Boolean = FALSE
  MUT endIndex AS Integer = index
  WHILE finished = FALSE
    LET nextIndex AS Integer = __json_skipWhitespace(bytes, idx)
    IF nextIndex >= len(bytes) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET quote AS Integer = toInt(collections::get(bytes, nextIndex))
    IF quote <> 34 THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET keyState AS __json_StringNode = __json_parseString(bytes, nextIndex + 1)
    LET colonIndex AS Integer = __json_skipWhitespace(bytes, keyState.index)
    IF colonIndex >= len(bytes) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET colon AS Integer = toInt(collections::get(bytes, colonIndex))
    IF colon <> 58 THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET valueState AS __json_Node = __json_parseValue(bytes, colonIndex + 1, depth)
    LET key AS String = keyState.value
    LET val AS Json = valueState.value
    acc = collections::set(acc, key, val)
    LET afterValue AS Integer = __json_skipWhitespace(bytes, valueState.index)
    IF afterValue >= len(bytes) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET code AS Integer = toInt(collections::get(bytes, afterValue))
    IF code = 44 THEN
      idx = afterValue + 1
    ELSEIF code = 125 THEN
      finished = TRUE
      endIndex = afterValue + 1
    ELSE
      FAIL error(77050003, "invalid JSON format")
    END IF
  END WHILE
  LET value AS Json = JsonObj[acc]
  RETURN __json_Node[value, endIndex]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseObjectItems", BODY));
}
