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
' bug-510 (DEC-04): scans bytes. A CR LF pair is ONE grapheme cluster, so the
' grapheme scan never saw either half as whitespace and a CRLF-formatted
' document was rejected; as bytes, 13 and 10 are each skipped.
FUNC __json_skipWhitespace(bytes AS List OF Byte, index AS Integer) AS Integer
  MUT at AS Integer = index
  LET n AS Integer = len(bytes)
  WHILE at < n
    IF NOT __json_isWhitespace(toInt(collections::get(bytes, at))) THEN
      RETURN at
    END IF
    at = at + 1
  END WHILE
  RETURN at
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_skipWhitespace", BODY));
}
