//! `__json_consumeDigits` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-302: iterative (see __json_skipWhitespace) — a long digit run recursed once
' per digit and overflowed the native stack.
' bug-510: over the token's bytes (48..57 are the ASCII digits), bounded by the
' token's end rather than the document's.
FUNC __json_consumeDigits(bytes AS List OF Byte, index AS Integer, endIndex AS Integer) AS Integer
  MUT at AS Integer = index
  WHILE at < endIndex
    LET code AS Integer = toInt(collections::get(bytes, at))
    IF code < 48 OR code > 57 THEN
      RETURN at
    END IF
    at = at + 1
  END WHILE
  RETURN at
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_consumeDigits", BODY));
}
