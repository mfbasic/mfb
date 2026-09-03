//! `__json_stringifyIndent` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-D: the indented renderer behind `json::stringify(value, indent)`.
' A depth-carrying clone of `__json_stringify`'s MATCH rather than a wrapper
' around it, because the layout decision (does this container expand?) happens
' per node and the compact body has nowhere to put it. The two leaf renderings
' are SHARED with the compact path -- `__json_stringifyNumber` and
' `__json_escapeString` -- so plan-120-C's byte shape is inherited here for
' free and cannot drift between the two forms.
'
' Layout is JavaScript's, exactly (captured from Node v24.12.0):
'   - one line per member, indent repeated once per depth level;
'   - `": "` after an object key, with the space;
'   - the closing bracket at the PARENT's depth;
'   - an empty array or object stays inline as `[]` / `{}` even in this mode,
'     and that applies at every depth -- a nested empty object does not expand
'     just because its parent did.
' The caller has already clamped `indent` and handled the compact cases, so an
' empty `indent` never reaches here.
FUNC __json_stringifyIndent(value AS Json, indent AS String, depth AS Integer) AS String
  MATCH value
    CASE JsonNull(nullValue)
      RETURN "null"
    CASE JsonBool(boolValue)
      IF boolValue.value THEN
        RETURN "true"
      END IF
      RETURN "false"
    CASE JsonNum(numValue)
      RETURN __json_stringifyNumber(numValue.value)
    CASE JsonStr(strValue)
      LET escaped AS String = __json_escapeString(strValue.value)
      LET withOpen AS String = "\"" & escaped
      RETURN withOpen & "\""
    CASE JsonArr(arrValue)
      IF len(arrValue.items) = 0 THEN
        RETURN "[]"
      END IF
      LET innerPad AS String = strings::repeat(indent, depth + 1)
      LET outerPad AS String = strings::repeat(indent, depth)
      MUT text AS String = "[\n"
      MUT first AS Boolean = TRUE
      FOR EACH item IN arrValue.items
        IF first THEN
          first = FALSE
        ELSE
          text = text & ",\n"
        END IF
        text = text & innerPad & __json_stringifyIndent(item, indent, depth + 1)
      NEXT
      LET arrClose AS String = "\n" & outerPad
      RETURN text & arrClose & "]"
    CASE JsonObj(objValue)
      IF len(objValue.fields) = 0 THEN
        RETURN "{}"
      END IF
      LET innerPad AS String = strings::repeat(indent, depth + 1)
      LET outerPad AS String = strings::repeat(indent, depth)
      MUT text AS String = "{\n"
      MUT first AS Boolean = TRUE
      FOR EACH entry IN objValue.fields
        IF first THEN
          first = FALSE
        ELSE
          text = text & ",\n"
        END IF
        LET escapedKey AS String = __json_escapeString(entry.key)
        LET keyText AS String = "\"" & escapedKey
        LET labelText AS String = keyText & "\": "
        LET valueText AS String = __json_stringifyIndent(entry.value, indent, depth + 1)
        text = text & innerPad & labelText & valueText
      NEXT
      LET objClose AS String = "\n" & outerPad
      RETURN text & objClose & "}"
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_stringifyIndent", BODY));
}
