//! `__json_parseUnicodeEscape` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseUnicodeEscape(bytes AS List OF Byte, index AS Integer) AS __json_StringNode
  ' plan-120-A: an unpaired surrogate is 77050025 ErrInvalidSurrogate, not the
  ' generic 77050003. The document's grammar is fine -- `\uD800` is a well-formed
  ' escape -- but MFB strings are Unicode text, so a half of a surrogate pair has
  ' no scalar to decode to. Naming it separately lets a caller tell "this JSON is
  ' malformed" apart from "this JSON carries a lone surrogate".
  LET first AS Integer = __json_parseHexQuad(bytes, index)
  IF __json_isHighSurrogate(first) THEN
    IF index + 10 >= len(bytes) THEN
      FAIL error(77050025, "invalid JSON string: high surrogate escape is not followed by a low surrogate escape")
    END IF
    IF toInt(collections::get(bytes, index + 4)) <> 92 THEN
      FAIL error(77050025, "invalid JSON string: high surrogate escape is not followed by a low surrogate escape")
    END IF
    IF toInt(collections::get(bytes, index + 5)) <> 117 THEN
      FAIL error(77050025, "invalid JSON string: high surrogate escape is not followed by a low surrogate escape")
    END IF
    LET second AS Integer = __json_parseHexQuad(bytes, index + 6)
    IF __json_isLowSurrogate(second) = FALSE THEN
      FAIL error(77050025, "invalid JSON string: high surrogate escape is not followed by a low surrogate escape")
    END IF
    LET codePoint AS Integer = 65536 + (first - 55296) * 1024 + (second - 56320)
    RETURN __json_StringNode[__json_codePointToString(codePoint), index + 10]
  END IF
  IF __json_isLowSurrogate(first) THEN
    FAIL error(77050025, "invalid JSON string: low surrogate escape has no preceding high surrogate escape")
  END IF
  RETURN __json_StringNode[__json_codePointToString(first), index + 4]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseUnicodeEscape", BODY));
}
