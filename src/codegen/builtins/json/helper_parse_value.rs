//! `__json_parseValue` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-510 (DEC-03/04): the scanners index the document's UTF-8 bytes, not a
' grapheme list. Every JSON structural character and whitespace is ASCII, so a
' byte compare is exact; a byte >= 128 only ever occurs inside a string (copied
' through verbatim) or inside a malformed number token (rejected by the grammar).
FUNC __json_parseValue(bytes AS List OF Byte, index AS Integer, depth AS Integer) AS __json_Node
  ' bug-422: structural nesting-depth guard. Each nested array/object descends
  ' one native frame group (parseValue -> parseArray/Object -> items -> parseValue)
  ' with no tail-call optimisation, so nesting depth = native stack depth. A ~1 KB
  ' document of nested `[` exhausted the stack somewhere between 800 and 1000
  ' frames and killed the process with an uncatchable SIGSEGV; json::parse is the
  ' untrusted-input boundary (HTTP bodies, files), so that was a remote crash.
  ' Capping here turns it into an ordinary catchable failure well before the stack
  ' runs out, mirroring the regex engine's __REGEX_DEPTH_LIMIT guard (bug-315).
  ' plan-120-A: reported as 77050024 ErrDepthExceeded rather than the generic
  ' 77050003, because the document is well-formed -- it is only nested deeper
  ' than this reader descends, which is a limit the caller can act on.
  IF depth > __JSON_DEPTH_LIMIT THEN
    FAIL error(77050024, "invalid JSON format: nested too deeply")
  END IF
  LET nextIndex AS Integer = __json_skipWhitespace(bytes, index)
  IF nextIndex >= len(bytes) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF

  LET code AS Integer = toInt(collections::get(bytes, nextIndex))
  IF code = 110 THEN
    LET endIndex AS Integer = __json_expectLiteral(bytes, nextIndex, "null")
    LET value AS Json = JsonNull[NOTHING]
    RETURN __json_Node[value, endIndex]
  ELSEIF code = 116 THEN
    LET endIndex AS Integer = __json_expectLiteral(bytes, nextIndex, "true")
    LET value AS Json = JsonBool[TRUE]
    RETURN __json_Node[value, endIndex]
  ELSEIF code = 102 THEN
    LET endIndex AS Integer = __json_expectLiteral(bytes, nextIndex, "false")
    LET value AS Json = JsonBool[FALSE]
    RETURN __json_Node[value, endIndex]
  ELSEIF code = 34 THEN
    LET parsed AS __json_StringNode = __json_parseString(bytes, nextIndex + 1)
    LET value AS Json = JsonStr[parsed.value]
    RETURN __json_Node[value, parsed.index]
  ELSEIF code = 91 THEN
    RETURN __json_parseArray(bytes, nextIndex + 1, depth + 1)
  ELSEIF code = 123 THEN
    RETURN __json_parseObject(bytes, nextIndex + 1, depth + 1)
  END IF

  RETURN __json_parseNumber(bytes, nextIndex)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseValue", BODY));
}
