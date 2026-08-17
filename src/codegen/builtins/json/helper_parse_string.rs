//! `__json_parseString` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Iterative string scan (plan-25-D §D3): accumulate each literal grapheme /
' decoded escape into a MUT chunk list and `strings::join` once at the end,
' replacing the former per-character `current & ch` recursion. The old form was
' O(n^2) (a fresh string of length k built at every step) and recursed one frame
' per character; this is O(n) with the in-place MUT append fast path. `current`
' seeds the builder so the observable result is byte-identical for any caller.
FUNC __json_parseString(chars AS List OF String, index AS Integer, current AS String) AS __json_StringNode
  MUT chunks AS List OF String = []
  IF current <> "" THEN
    chunks = collections::append(chunks, current)
  END IF
  MUT idx AS Integer = index
  MUT finished AS Boolean = FALSE
  MUT endIndex AS Integer = index
  WHILE finished = FALSE
    IF idx >= len(chars) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET ch AS String = collections::get(chars, idx)
    IF ch = "\"" THEN
      finished = TRUE
      endIndex = idx + 1
    ELSEIF ch = "\\" THEN
      IF idx + 1 >= len(chars) THEN
        FAIL error(77050003, "invalid JSON format")
      END IF
      LET escapeState AS __json_StringNode = __json_parseEscape(chars, idx + 1)
      chunks = collections::append(chunks, escapeState.value)
      idx = escapeState.index
    ELSEIF __json_isRawControlChar(ch) THEN
      FAIL error(77050003, "invalid JSON format")
    ELSE
      chunks = collections::append(chunks, ch)
      idx = idx + 1
    END IF
  END WHILE
  LET result AS String = strings::join(chunks, "")
  RETURN __json_StringNode[result, endIndex]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseString", BODY));
}
