//! `__json_parseUnicodeEscape` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseUnicodeEscape(chars AS List OF String, index AS Integer) AS __json_StringNode
  LET first AS Integer = __json_parseHexQuad(chars, index)
  IF __json_isHighSurrogate(first) THEN
    IF index + 10 >= len(chars) THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    IF collections::get(chars, index + 4) <> "\\" THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    IF collections::get(chars, index + 5) <> "u" THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET second AS Integer = __json_parseHexQuad(chars, index + 6)
    IF __json_isLowSurrogate(second) = FALSE THEN
      FAIL error(77050003, "invalid JSON format")
    END IF
    LET codePoint AS Integer = 65536 + (first - 55296) * 1024 + (second - 56320)
    RETURN __json_StringNode[__json_codePointToString(codePoint), index + 10]
  END IF
  IF __json_isLowSurrogate(first) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  RETURN __json_StringNode[__json_codePointToString(first), index + 4]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseUnicodeEscape", BODY));
}
