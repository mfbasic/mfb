//! `json::stringify` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:stringify@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_stringify(value AS Json) AS String
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
      MUT text AS String = "["
      MUT first AS Boolean = TRUE
      FOR EACH item IN arrValue.items
        IF first THEN
          first = FALSE
        ELSE
          text = text & ","
        END IF
        text = text & __json_stringify(item)
      NEXT
      RETURN text & "]"
    CASE JsonObj(objValue)
      MUT text AS String = "{"
      MUT first AS Boolean = TRUE
      FOR EACH entry IN objValue.fields
        IF first THEN
          first = FALSE
        ELSE
          text = text & ","
        END IF
        LET escapedKey AS String = __json_escapeString(entry.key)
        LET keyText AS String = "\"" & escapedKey
        LET labelText AS String = keyText & "\":"
        LET valueText AS String = __json_stringify(entry.value)
        text = text & labelText & valueText
      NEXT
      RETURN text & "}"
  END MATCH
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_STRINGIFY,
    return_type: ReturnType::Fixed("String"),
}];

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const STRINGIFY: BuiltinFunction =
    BuiltinFunction::mfb("json.stringify", "stringify", INTRO, DESC, &[], OV, BODY);
