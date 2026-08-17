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
FUNC __json_consumeDigits(chars AS List OF String, index AS Integer) AS Integer
  MUT at AS Integer = index
  WHILE at < len(chars)
    IF NOT __json_isDigit(collections::get(chars, at)) THEN
      RETURN at
    END IF
    at = at + 1
  END WHILE
  RETURN at
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_consumeDigits", BODY));
}
