//! `json::stringify` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:stringify@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

const INTRO: &str = r#"Serialize a `Json` value as compact JSON text"#;

const DESC: &str = r#"`json::stringify` serializes `value` into a single JSON document with no
indentation, no line breaks, and no whitespace between tokens. Arrays and objects
are serialized recursively, so one call renders a whole tree.

Each variant maps to its JSON form: `JsonNull` emits `null`; `JsonBool` emits
`true` or `false`; `JsonStr` emits a double-quoted escaped string; `JsonArr`
emits `[`, its items in list order separated by `,`, and `]`; `JsonObj` emits
`{`, its members as `"key":value` separated by `,`, and `}`, with each key
escaped as a JSON string. An empty `JsonArr` emits `[]` and an empty `JsonObj`
emits `{}`. Object members are emitted in the map's iteration order, which is not
guaranteed to be insertion order or sorted order — do not rely on it for stable
output or for byte-comparison of two documents.

**String escaping.** `"` and `\` are escaped, and so is `/` — every forward slash
is emitted as `\/`. That is valid JSON and parses back identically, but it means
`json::stringify` output is not byte-identical to what most other JSON writers
produce. The C0 escapes `\b`, `\t`, `\n`, `\f`, and `\r` are used where they
apply, and any remaining control character below code point `32` is emitted as a
`\u00XX` escape. Everything else, including all non-ASCII text, is emitted
literally as UTF-8 rather than as `\u` escapes.

**Numbers.** A `JsonNum` holds a `Float`, and the rendering is chosen so that it
round-trips: the whole-number form is tried first, so an integral value emits as
`100` rather than `100.0`, and otherwise the *shortest* fractional rendering that
parses back to exactly the same `Float` is searched for and used. The round trip
is verified rather than assumed — `3.141592653589793` serializes with all of its
digits intact, not truncated to a fixed precision. If no rendering round-trips,
the call fails rather than emitting a silently lossy number.

The number path also guards against a non-finite `Float`, but that guard is
unreachable from ordinary MFBASIC code: no user-accessible `Float` is non-finite,
because storing a `NaN` or an infinity into a record field is an observation
boundary that fails first with `ErrFloatNaN` or `ErrFloatOverflow`. A `JsonNum`
therefore cannot be constructed around a non-finite value in the first place.

The argument accepts the `Json` union or any one of its six member types
(`JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, `JsonObj`) directly, so
a scalar member value can be serialized without wrapping it.

The output is always re-readable by `json::parse`, which makes
`parse`/`stringify` a lossless round trip for every value `parse` can produce."#;

const EX: &str = r#"Round-trip a document through text:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"name\":\"Ada\",\"n\":3}")
  io::print(json::stringify(doc))
END SUB
```

Serialize a member type directly, without wrapping it in the union:

```
IMPORT json
IMPORT io

SUB main
  io::print(json::stringify(JsonBool[TRUE]))
  io::print(json::stringify(JsonStr["a/b"]))
END SUB
```

Build a value and serialize it:

```
IMPORT json
IMPORT io

SUB main
  LET items AS List OF json::Json = [JsonNum[1.0], JsonNum[2.5]]
  io::print(json::stringify(JsonArr[items]))
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
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

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stringify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                    desc: "The value to read from. Accepts the Json union or any of JsonNull, JsonBool, JsonNum, JsonStr, JsonArr, JsonObj; traversal only succeeds through JsonObj members.",
                aliases: &[],
                ty: ParameterType::Named("Json"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(FUNC_BODY, "__json_stringify"),
        }],
    });
}
