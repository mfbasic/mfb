//! `__json_numberEnd` — shared private helper for the `json` package.
//!
//! Was `__json_collectNumber`, which returned the token as a `String` inside a
//! `__json_StringNode`. bug-510 (DEC-03): the token is now validated over its
//! bytes before anything is sliced, so this only finds where it ends; the caller
//! decodes the (by then known-ASCII) token once, and the record is gone.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-302: iterative (see __json_skipWhitespace) — a long numeric literal recursed
' once per character and overflowed the native stack.
' The end of the number token starting at `index`: the first `,` (44), `]` (93),
' `}` (125) or whitespace byte, or the end of the document. All four terminators
' are ASCII, so the token never ends inside a multi-byte scalar; a non-ASCII byte
' inside it is carried to __json_validNumber and rejected there.
FUNC __json_numberEnd(bytes AS List OF Byte, index AS Integer) AS Integer
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
  RETURN at
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_numberEnd", BODY));
}
