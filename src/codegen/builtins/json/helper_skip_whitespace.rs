//! `__json_skipWhitespace` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-302: iterative, not recursive. MFBASIC has no tail-call optimization, so
' `RETURN __json_skipWhitespace(chars, index + 1)` consumed one native frame per
' input character and a long whitespace run overflowed the stack (SIGSEGV) on a
' payload well under the HTTP request cap. The array/object parsers were already
' rewritten iteratively for exactly this reason; the scalar scanners were missed.
FUNC __json_skipWhitespace(chars AS List OF String, index AS Integer) AS Integer
  MUT at AS Integer = index
  WHILE at < len(chars)
    IF NOT __json_isWhitespace(collections::get(chars, at)) THEN
      RETURN at
    END IF
    at = at + 1
  END WHILE
  RETURN at
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_skipWhitespace", BODY));
}
