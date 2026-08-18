//! `__json_parseObjectItems` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseObjectItems(chars AS List OF String, index AS Integer, fields AS Map OF String TO Json, depth AS Integer) AS __json_Node
  ' Iterative accumulation, mirroring the array parser. `acc = set(acc, key, val)`
  ' uses the in-place MUT map set once Phase 3 lands; today it is the rebuild path
  ' (no regression). Eliminating the recursion also avoids deep call stacks on
  ' large objects (plan-02 Phase 4 fallback).
  MUT acc AS Map OF String TO Json = fields
  MUT idx AS Integer = index
  MUT finished AS Boolean = FALSE
  MUT endIndex AS Integer = index
  WHILE finished = FALSE
    LET nextIndex AS Integer = __json_skipWhitespace(chars, idx)
    IF nextIndex >= len(chars) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET quote AS String = collections::get(chars, nextIndex)
    IF quote <> "\"" THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET keyState AS __json_StringNode = __json_parseString(chars, nextIndex + 1, "")
    LET colonIndex AS Integer = __json_skipWhitespace(chars, keyState.index)
    IF colonIndex >= len(chars) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET colon AS String = collections::get(chars, colonIndex)
    IF colon <> ":" THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET valueState AS __json_Node = __json_parseValue(chars, colonIndex + 1, depth)
    LET key AS String = keyState.value
    LET val AS Json = valueState.value
    acc = collections::set(acc, key, val)
    LET afterValue AS Integer = __json_skipWhitespace(chars, valueState.index)
    IF afterValue >= len(chars) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET ch AS String = collections::get(chars, afterValue)
    IF ch = "," THEN
      idx = afterValue + 1
    ELSEIF ch = "}" THEN
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
