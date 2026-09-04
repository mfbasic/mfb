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
' bug-510: measures the token over bytes, then copies it out in ONE slice rather
' than growing an accumulator per character. The four terminators (`,` `]` `}` and
' whitespace) are ASCII, so the token never ends inside a multi-byte scalar and
' always decodes; a non-ASCII scalar inside the token is carried through and
' rejected by __json_validNumber, exactly as the grapheme scan rejected it.
FUNC __json_collectNumber(bytes AS List OF Byte, index AS Integer) AS __json_StringNode
  MUT at AS Integer = index
  MUT finished AS Boolean = FALSE
  MUT code AS Integer = 0
  LET n AS Integer = len(bytes)
  WHILE finished = FALSE AND at < n
    code = toInt(collections::get(bytes, at))
    IF code = 44 THEN
      finished = TRUE
    ELSEIF code = 93 THEN
      finished = TRUE
    ELSEIF code = 125 THEN
      finished = TRUE
    ELSEIF __json_isWhitespace(code) THEN
      finished = TRUE
    ELSE
      at = at + 1
    END IF
  END WHILE
  LET token AS String = encoding::utf8Decode(collections::mid(bytes, index, at - index))
  RETURN __json_StringNode[token, at]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_collectNumber", BODY));
}
