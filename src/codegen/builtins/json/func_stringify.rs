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

const INTRO: &str = r#"Serialize a `json::Json` value as JSON text, compact or indented"#;

const DESC: &str = r#"`json::stringify` serializes `value` into a single JSON document. Called with one
argument it produces compact text: no indentation, no line breaks, and no
whitespace between tokens. Arrays and objects are serialized recursively, so one
call renders a whole tree.

**Indented output.** Pass a second argument to spread the document over multiple
lines — either a number of spaces to indent each level by, or the text to indent
with:

```
json::stringify(doc, 2)     ' two spaces per level
json::stringify(doc, "\t")  ' one tab per level
```

The layout is JavaScript's, exactly: one member per line, the indent repeated
once per level, `": "` after each object key, and the closing bracket back at the
parent's indentation. An empty array or object stays on one line as `[]` or `{}`
even in this mode, at any depth. For the same tree, this produces the same bytes
as `JSON.stringify(value, null, space)` does in Node.

The second argument's limits are JavaScript's too. A count is clamped to 0
through 10, and a text indent is truncated to its first 10 characters — a larger
request is not an error, it simply stops growing. A count of `0` (or a negative
one) and an empty text both mean "compact", producing exactly what the one-argument
form produces, so a program can decide at run time whether to indent without
branching:

```
LET pretty AS Integer = 2   ' or 0 to turn indenting off
io::print(json::stringify(doc, pretty))
```

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

Where two equally short renderings both read back exactly, the one whose last
digit is even is chosen, as `JSON.stringify` does. It is a rare tie, and the
alternative rule — round half away from zero — disagrees with JavaScript on a
small fraction of values.

Negative zero emits as `0`, not `-0`, matching `JSON.stringify` — JSON has no
way to mark a zero's sign that readers agree on, and the same information is lost
by a JavaScript round trip. `toString(-0.0)` is unaffected and still shows the
sign; this rule applies only to JSON output.

Numbers are written exactly as `JSON.stringify` writes them, which decides
between a plain decimal and an exponential form by magnitude alone: plain while
`1e-6 <= |value| < 1e21`, exponential outside it. So `1e20` emits
`100000000000000000000` and `1e21` emits `1e+21`; `0.000001` stays plain and
`1e-7` does not. The exponent carries an explicit sign and is never padded, so
it reads `1e+21` and `1e-7`.

**Every finite `Float` has a rendering.** The digits come from the value's
significant digits rather than from a fixed number of decimal places, so
magnitude is no longer a reason a number cannot be written: `1e-30` emits
`1e-30` and the smallest subnormal emits `5e-324`. Both of those used to fail
outright.

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
```

Indent by two spaces:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"name\":\"Ada\",\"tags\":[\"x\",\"y\"]}")
  io::print(json::stringify(doc, 2))
END SUB
```

prints:

```
{
  "name": "Ada",
  "tags": [
    "x",
    "y"
  ]
}
```

Indent with a tab instead, and note that an empty container stays on one line:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"a\":[],\"b\":{\"c\":1}}")
  io::print(json::stringify(doc, "\t"))
END SUB
```

prints:

```
{
	"a": [],
	"b": {
		"c": 1
	}
}
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

/// plan-120-D: the `stringify(value, indent AS Integer)` overload. Clamps to
/// JavaScript's 0..=10 and falls back to the compact body at 0, so the 1-arg
/// output stays reachable through the 2-arg form without duplicating it.
#[rustfmt::skip]
const FUNC_BODY_COUNT: &str =
r#"FUNC __json_stringifyCount(value AS Json, indent AS Integer) AS String
  LET pad AS String = __json_indentFromCount(indent)
  IF pad = "" THEN
    RETURN __json_stringify(value)
  END IF
  RETURN __json_stringifyIndent(value, pad, 0)
END FUNC"#;

/// plan-120-D: the `stringify(value, indent AS String)` overload. Same shape as
/// the Integer form; an empty indent (after clamping) means compact, which is
/// what `JSON.stringify(v, null, "")` does.
#[rustfmt::skip]
const FUNC_BODY_TEXT: &str =
r#"FUNC __json_stringifyText(value AS Json, indent AS String) AS String
  LET pad AS String = __json_indentFromText(indent)
  IF pad = "" THEN
    RETURN __json_stringify(value)
  END IF
  RETURN __json_stringifyIndent(value, pad, 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stringify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        // plan-120-D: three overloads whose positional layouts differ, so the
        // per-position render would only show the first. The `"or"`-joined
        // string names all three forms (the net/audio overloaded idiom).
        expected_arguments: Some("Json or Json, Integer or Json, String"),
        internal_only: false,
        implementations: vec![
            // Compact form. Untouched by plan-120-D — the pretty overloads route
            // back into this body when their indent clamps to empty, so there is
            // exactly one compact renderer and it cannot drift from itself.
            Implementation {
                params: vec![value_param()],
                return_type: ParameterType::String,
                // plan-120-A: this was empty, so the page rendered no Errors
                // section at all even though `stringify` could fail.
                //
                // plan-120-G: `ErrInvalidFormat` is no longer reachable. It used
                // to fire whenever no fixed-point rendering round-tripped — any
                // Float too small to reach a significant digit in 25 places, such
                // as 1e-30 — and the significant-digit renderer always finds one
                // within 17 digits. The `FAIL` survives as an invariant guard, so
                // the code stays listed rather than becoming an undocumented way
                // for the call to fail; it simply should never be seen. The two
                // non-finite codes were already in that position (see the DESC).
                errors: vec!["ErrInvalidFormat", "ErrFloatNaN", "ErrFloatInf"],
                body: Body::mfb(FUNC_BODY, "__json_stringify"),
            },
            // plan-120-D: indent by a number of spaces. Same arity as the String
            // form below, distinguished by parameter TYPE — the `crypto::hash`
            // pattern (`(Hash, List OF Byte)` vs `(Hash, String)`), not
            // `datetime::parse`'s arity split. Each carries its own `Body::mfb`
            // function name, so unlike two `AbiFunction` overloads they cannot
            // collapse onto one helper symbol.
            Implementation {
                params: vec![
                    value_param(),
                    Parameter {
                        name: "indent",
                        desc: "How many spaces to indent each level by. Clamped to 0 through 10, as in JavaScript; 0 produces the compact form.",
                        aliases: &["space"],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::String,
                errors: vec!["ErrInvalidFormat", "ErrFloatNaN", "ErrFloatInf"],
                body: Body::mfb(FUNC_BODY_COUNT, "__json_stringifyCount"),
            },
            // plan-120-D: indent by a literal string, e.g. a tab.
            Implementation {
                params: vec![
                    value_param(),
                    Parameter {
                        name: "indent",
                        desc: "The text to indent each level with, such as a tab. Truncated to its first 10 characters, as in JavaScript; an empty string produces the compact form.",
                        aliases: &["space"],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::String,
                errors: vec!["ErrInvalidFormat", "ErrFloatNaN", "ErrFloatInf"],
                body: Body::mfb(FUNC_BODY_TEXT, "__json_stringifyText"),
            },
        ],
    });
}

/// The `value` parameter, identical across all three overloads.
fn value_param() -> Parameter {
    Parameter {
        name: "value",
        desc: "The value to serialize. Accepts the Json union or any of JsonNull, JsonBool, JsonNum, JsonStr, JsonArr, JsonObj; arrays and objects are serialized recursively, so one call renders a whole tree.",
        aliases: &[],
        ty: ParameterType::named("Json"),
        default: DefaultValue::None,
    }
}
