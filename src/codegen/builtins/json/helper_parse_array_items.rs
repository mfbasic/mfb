//! `__json_parseArrayItems` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseArrayItems(bytes AS List OF Byte, index AS Integer, items AS List OF Json, depth AS Integer) AS __json_Node
  ' Iterative accumulation so `acc = collections::append(acc, item)` hits the
  ' in-place MUT append (plan-02 Phase 4 fallback): O(n) instead of the O(n^2)
  ' functional `LET next = append(items, ...)` that bound a fresh name every
  ' element. The typed `LET item` lets the in-place gate classify the element.
  MUT acc AS List OF Json = items
  MUT idx AS Integer = index
  MUT finished AS Boolean = FALSE
  MUT endIndex AS Integer = index
  WHILE finished = FALSE
    LET parsed AS __json_Node = __json_parseValue(bytes, idx, depth)
    LET item AS Json = parsed.value
    acc = collections::append(acc, item)
    LET nextIndex AS Integer = __json_skipWhitespace(bytes, parsed.index)
    IF nextIndex >= len(bytes) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET code AS Integer = toInt(collections::get(bytes, nextIndex))
    IF code = 44 THEN
      idx = nextIndex + 1
    ELSEIF code = 93 THEN
      finished = TRUE
      endIndex = nextIndex + 1
    ELSE
      FAIL error(77050003, "invalid JSON format")
    END IF
  END WHILE
  LET value AS Json = JsonArr[acc]
  RETURN __json_Node[value, endIndex]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseArrayItems", BODY));
}
