//! `__json_parseValue` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parseValue(chars AS List OF String, index AS Integer, depth AS Integer) AS __json_Node
  ' bug-422: structural nesting-depth guard. Each nested array/object descends
  ' one native frame group (parseValue -> parseArray/Object -> items -> parseValue)
  ' with no tail-call optimisation, so nesting depth = native stack depth. A ~1 KB
  ' document of nested `[` exhausted the stack somewhere between 800 and 1000
  ' frames and killed the process with an uncatchable SIGSEGV; json::parse is the
  ' untrusted-input boundary (HTTP bodies, files), so that was a remote crash.
  ' Capping here turns it into an ordinary catchable failure well before the stack
  ' runs out, mirroring the regex engine's __REGEX_DEPTH_LIMIT guard (bug-315).
  IF depth > __JSON_DEPTH_LIMIT THEN
    FAIL error(77050003, "invalid JSON format: nested too deeply")
  END IF
  LET nextIndex AS Integer = __json_skipWhitespace(chars, index)
  IF nextIndex >= len(chars) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF

  LET ch AS String = collections::get(chars, nextIndex)
  IF ch = "n" THEN
    LET endIndex AS Integer = __json_expectLiteral(chars, nextIndex, "null")
    LET value AS Json = JsonNull[NOTHING]
    RETURN __json_Node[value, endIndex]
  ELSEIF ch = "t" THEN
    LET endIndex AS Integer = __json_expectLiteral(chars, nextIndex, "true")
    LET value AS Json = JsonBool[TRUE]
    RETURN __json_Node[value, endIndex]
  ELSEIF ch = "f" THEN
    LET endIndex AS Integer = __json_expectLiteral(chars, nextIndex, "false")
    LET value AS Json = JsonBool[FALSE]
    RETURN __json_Node[value, endIndex]
  ELSEIF ch = "\"" THEN
    LET parsed AS __json_StringNode = __json_parseString(chars, nextIndex + 1, "")
    LET value AS Json = JsonStr[parsed.value]
    RETURN __json_Node[value, parsed.index]
  ELSEIF ch = "[" THEN
    RETURN __json_parseArray(chars, nextIndex + 1, depth + 1)
  ELSEIF ch = "{" THEN
    RETURN __json_parseObject(chars, nextIndex + 1, depth + 1)
  END IF

  RETURN __json_parseNumber(chars, nextIndex)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_parseValue", BODY));
}
