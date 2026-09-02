//! `json::stringify` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:stringify@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Serialize a `json::Json` value as compact JSON text"#;

const DESC: &str = r#"`json::stringify` serializes `value` into a single JSON document with no
indentation, no line breaks, and no whitespace between tokens. Arrays and objects
are serialized recursively, so one call renders a whole tree.

Each variant maps to its JSON form: `json::JsonNull` emits `null`; `json::JsonBool` emits
`true` or `false`; `json::JsonStr` emits a double-quoted escaped string; `json::JsonArr`
emits `[`, its items in list order separated by `,`, and `]`; `json::JsonObj` emits
`{`, its members as `"key":value` separated by `,`, and `}`, with each key
escaped as a JSON string. An empty `json::JsonArr` emits `[]` and an empty `json::JsonObj`
emits `{}`.

**Object member order.** Members are emitted in the order the `json::JsonObj`'s
map holds them, which for a document that came from `json::parse` is the order
the members appeared in the text. So `json::stringify(json::parse(doc))` returns
the members in the document's own order, and round-tripping a document through
this pair does not shuffle it.

This is a promise `json` makes and pins with its own tests, not a promise the
`Map` type makes — `mfb man types map` still describes iteration order as
implementation-defined. Note it differs from JavaScript, which emits keys that
look like array indexes first, in ascending numeric order:
`JSON.stringify({b:1, a:2, "10":3, "2":4})` gives `{"2":4,"10":3,"b":1,"a":2}`
in Node, where `json::stringify` keeps `{"b":1,"a":2,"10":3,"2":4}`. MFBASIC has
no JavaScript object semantics to justify that reordering, so it does not copy it.

**String escaping.** `"` and `\` are escaped. `/` is **not** — a forward slash is
emitted literally. RFC 8259 permits `\/` but never requires it, and no widely-used
writer emits it, so a literal `/` is what makes `json::stringify` output comparable
byte-for-byte with other producers. (`json::parse` still accepts `\/` on input, as
it must.) The C0 escapes `\b`, `\t`, `\n`, `\f`, and `\r` are used where they
apply, and any remaining control character below code point `32` is emitted as a
`\u00XX` escape. Everything else, including all non-ASCII text, is emitted
literally as UTF-8 rather than as `\u` escapes.

**Numbers.** A `json::JsonNum` holds a `Float`, and the rendering is chosen so that it
round-trips: the whole-number form is tried first, so an integral value emits as
`100` rather than `100.0`, and otherwise the *shortest* fractional rendering that
parses back to exactly the same `Float` is searched for and used. The round trip
is verified rather than assumed — `3.141592653589793` serializes with all of its
digits intact, not truncated to a fixed precision.

Negative zero emits as `0`, not `-0`, matching `JSON.stringify` — JSON has no
way to mark a zero's sign that readers agree on, and the same information is lost
by a JavaScript round trip. `toString(-0.0)` is unaffected and still shows the
sign; this rule applies only to JSON output.

The renderings searched are fixed-point ones of up to 25 decimal places, so a
`Float` too small to reach its first significant digit within 25 places has no
candidate that round-trips — `1e-30` and `5e-324` are the sort of value this
reaches. Rather than emit a silently lossy number, the call fails with
`errorCode::ErrInvalidFormat`. Such a value parses in and can be read back out of
a `json::Json`; it is only writing it as text that has no answer here.

The number path also guards against a non-finite `Float`, failing with
`errorCode::ErrFloatNaN` for a `NaN` and `errorCode::ErrFloatInf` for an infinity.
That guard is unreachable from ordinary MFBASIC code: no user-accessible `Float`
is non-finite, because storing a `NaN` or an infinity into a record field is an
observation boundary that fails first with `ErrFloatNaN` or `ErrFloatOverflow`. A
`json::JsonNum` therefore cannot be constructed around a non-finite value in the
first place, and the guard stands only so that any future path that could reach
it says which non-finite value it was.

The argument accepts the `json::Json` union or any one of its six member types
(`json::JsonNull`, `json::JsonBool`, `json::JsonNum`, `json::JsonStr`, `json::JsonArr`, `json::JsonObj`) directly, so
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
  io::print(json::stringify(json::JsonBool[TRUE]))
  io::print(json::stringify(json::JsonStr["a/b"]))
END SUB
```

Build a value and serialize it:

```
IMPORT json
IMPORT io

SUB main
  LET items AS List OF json::Json = [json::JsonNum[1.0], json::JsonNum[2.5]]
  io::print(json::stringify(json::JsonArr[items]))
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stringify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The value to serialize. Accepts the Json union or any of JsonNull, JsonBool, JsonNum, JsonStr, JsonArr, JsonObj; arrays and objects are serialized recursively, so one call renders a whole tree.",
                aliases: &[],
                ty: ParameterType::named("Json"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            // plan-120-A: this was empty, so the page rendered no Errors section
            // at all even though `stringify` has always been able to fail —
            // `ErrInvalidFormat` when no fixed-point rendering round-trips (a
            // Float too small to reach a significant digit in 25 places, e.g.
            // 1e-30). The two non-finite codes are unreachable from ordinary
            // MFBASIC (see the DESC) but are what the guard raises, and a
            // handler matching on them should be able to find them documented.
            errors: vec!["ErrInvalidFormat", "ErrFloatNaN", "ErrFloatInf"],
            body: Body::mfb(FUNC_BODY, "__json_stringify"),
        }],
    });
}
