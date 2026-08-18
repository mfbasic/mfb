//! `__json_collectNumber` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-302: iterative (see __json_skipWhitespace) — a long numeric literal recursed
' once per character and overflowed the native stack.
FUNC __json_collectNumber(chars AS List OF String, index AS Integer, current AS String) AS __json_StringNode
  MUT at AS Integer = index
  MUT acc AS String = current
  WHILE at < len(chars)
    LET ch AS String = collections::get(chars, at)
    IF ch = "," THEN
      RETURN __json_StringNode[acc, at]
    END IF
    IF ch = "]" THEN
      RETURN __json_StringNode[acc, at]
    END IF
    IF ch = "}" THEN
      RETURN __json_StringNode[acc, at]
    END IF
    IF __json_isWhitespace(ch) THEN
      RETURN __json_StringNode[acc, at]
    END IF
    acc = acc & ch
    at = at + 1
  END WHILE
  RETURN __json_StringNode[acc, at]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_collectNumber", BODY));
}
