//! `json::parse` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:parse@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Parse a complete JSON document from text into a `json::Json` value"#;

const DESC: &str = r#"`json::parse` reads exactly one complete JSON document from `value` and returns
it as a `json::Json` union value. Leading and trailing JSON whitespace is skipped, and
anything other than whitespace after the first complete document is rejected — so
a string holding two documents, or a document followed by stray text, fails
rather than parsing the first and ignoring the rest.

Whitespace means exactly the four characters JSON allows: space, tab, carriage
return, and line feed. No other character is skippable, anywhere.

The input is scanned byte by byte over its UTF-8 encoding. Every JSON structural
character and every whitespace character is ASCII, so the scan never splits a
multi-byte scalar: text inside a string, including combining marks and characters
outside the Basic Multilingual Plane, is copied through exactly as written, and a
carriage return followed by a line feed is two whitespace characters, as JSON
defines them. Each JSON form maps to one variant of the `json::Json` union:

- `null` becomes `json::JsonNull[NOTHING]`.
- `true` and `false` become `json::JsonBool`.
- A number becomes `json::JsonNum`, holding a `Float`.
- A string becomes `json::JsonStr`.
- An array becomes `json::JsonArr`, holding a `List OF json::Json`; `[]` yields an empty list.
- An object becomes `json::JsonObj`, holding a `Map OF String TO json::Json`; `{}` yields an
  empty map. Duplicate keys collapse last-wins, because each pair is written into
  the map as it is read.

**Strings.** The escapes `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t`, and
`\uXXXX` are decoded. A `\u` escape must be exactly four hex digits; a high
surrogate must be followed immediately by `\u` and a low surrogate, and the pair
is combined into one code point. An unknown escape letter, a truncated escape, or
a code point outside `0`–`1114111` is rejected with `errorCode::ErrInvalidFormat`.
A raw control character (code point below `32`) that appears unescaped inside a
string is also rejected, as JSON requires.

An unpaired surrogate — a lone low surrogate, or a high surrogate not followed by
a low one — gets its own code, `errorCode::ErrInvalidSurrogate`. Such an escape is
well-formed JSON; what it names is half of a code point rather than a whole one,
and an MFBASIC `String` holds Unicode text, so there is nothing to decode it to.
A document from a system that emits lone surrogates must be repaired before it
can be read here, and the separate code is what tells you that is the problem.

**Numbers.** The token is validated against the JSON number grammar before it is
converted: an optional leading `-`, then either a single `0` or a nonzero digit
followed by further digits, then an optional `.` with at least one digit, then an
optional `e`/`E` with an optional sign and at least one digit. A leading `+`, a
leading `.`, a trailing `.`, a superfluous leading zero such as `01`, and the
JavaScript spellings `NaN` and `Infinity` are all rejected — all with
`errorCode::ErrInvalidFormat`, because those are grammar mistakes.

The accepted token is then converted to a `Float` (IEEE 754 binary64), and the two
ways that can go wrong are reported differently:

- **More precision than binary64 carries** is approximated, not rejected.
  `3.14159265358979311599796346854` parses; it becomes the nearest `Float`.
- **More magnitude than binary64 carries** is rejected. `1e400` is a valid JSON
  number with no `Float` anywhere near it, and MFBASIC has no observable infinity
  to stand in for it, so the conversion's own verdict is raised —
  `errorCode::ErrOverflow` here. Whatever `toFloat` would say about the same text
  is what `json::parse` says, so a `TRAP` can tell "this number does not fit" from
  "this is not JSON" by the code alone.

This is a real difference from JavaScript, which turns `1e400` into `Infinity`.

The parser is iterative rather than recursive at every *scanning* level —
whitespace runs, digit runs, string bodies, and the sibling elements of a single
array or object — so a long flat document does not use a stack frame
per character.

Structural *nesting*, by contrast, does descend one call per level, so it is
bounded: a document nested beyond a fixed structural depth (256 levels of arrays
and objects combined) is rejected with `errorCode::ErrDepthExceeded` rather than
being allowed to exhaust the native stack. Any realistic document is far within
this limit; the cap exists only so that adversarial deeply-nested input fails
cleanly instead of crashing the process. The code is separate from
`ErrInvalidFormat` on purpose — the text is well-formed JSON, it is simply nested
deeper than this reader descends, which is something a caller can act on.

**The reviver form.** `json::parse(value, reviver)` parses exactly as above, then
walks the finished document, calling `reviver` once for every value in it. What
the reviver returns is stored in place of what it was given, so a single pass can
convert dates, unwrap tagged objects, or normalize numbers without traversing the
document a second time.

The walk is innermost-first. Every element of an array and every member of an
object is revived before the array or object containing it, and the container the
reviver receives already holds the revived children rather than the original ones.
The document root is revived last of all.

The key says where the value came from:

- inside an object, the member name;
- inside an array, the index written as a decimal string — `"0"`, `"1"`, and so on;
- for the document root, the empty string `""`. That call is the only one that
  sees the whole document, which is where a reviver that finishes the document as
  a whole does its work.

Duplicate keys have already collapsed last-wins by the time revival starts, so the
reviver is called once per surviving member, not once per occurrence in the text.

The document is parsed completely before any revival happens. Malformed text fails
without the reviver being called at all, so a reviver only ever receives a value
that parsed. An error the reviver itself raises is not caught: it surfaces at the
`json::parse` call site carrying the code and message the reviver used.

This is `JSON.parse`'s second argument, with one difference. JavaScript removes a
member whose reviver returns `undefined`; MFBASIC has no `undefined`, so there is
nothing to ask for removal with and every return is stored. Returning
`json::JsonNull[NOTHING]` therefore stores a JSON null instead of dropping the
member.

The argument may also be passed by the name `text`."#;

const EX: &str = r#"Parse an object and read a nested value out of it:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"ok\":true,\"items\":[1,2,3]}")
  io::print(json::stringify(json::get(doc, ["ok"])))
END SUB
```

Pass the argument by name:

```
IMPORT json
IMPORT io

SUB main
  LET empty AS json::Json = json::parse(text := "null")
  io::print(json::stringify(empty))
END SUB
```

Handle malformed input instead of failing:

```
IMPORT json
IMPORT io

FUNC parseOrNull(text AS String) AS json::Json
  RETURN json::parse(text)
  TRAP(e)
    RETURN json::JsonNull[NOTHING]
  END TRAP
END FUNC

SUB main
  io::print(json::stringify(parseOrNull("[1,2]")))
  io::print(json::stringify(parseOrNull("not json")))
END SUB
```

Transform every value as the document is read, using the reviver form:

```
IMPORT json
IMPORT io

' Double every number, wherever it sits in the document.
FUNC double(key AS String, value AS json::Json) AS json::Json
  MATCH value
    CASE json::JsonNum(n)
      RETURN json::JsonNum[n.value * 2.0]
    CASE ELSE
      RETURN value
  END MATCH
END FUNC

SUB main
  io::print(json::stringify(json::parse("{\"a\":1,\"b\":[2,3]}", double)))
END SUB
```

Act on one member by name, and on the finished document under the empty key:

```
IMPORT json
IMPORT io

FUNC summarize(key AS String, value AS json::Json) AS json::Json
  IF key = "" THEN
    RETURN json::JsonStr["document: " & json::stringify(value)]
  END IF
  IF key = "name" THEN
    RETURN json::JsonStr["Ada"]
  END IF
  RETURN value
END FUNC

SUB main
  io::print(json::stringify(json::parse("{\"name\":\"?\",\"id\":7}", summarize)))
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __json_parse(value AS String) AS Json
  ' bug-510 (DEC-03/04): tokenise the UTF-8 bytes, not a grapheme list. The
  ' list is one tight byte per byte; every structural character and whitespace
  ' is ASCII, so byte compares are exact and a CR LF pair is two whitespace
  ' bytes rather than one opaque cluster. Indices below are byte offsets.
  LET bytes AS List OF Byte = strings::toBytes(value)
  ' bug-422: depth 0 seeds the structural nesting-depth guard threaded through
  ' the value/array/object parsers below.
  LET parsed AS __json_Node = __json_parseValue(bytes, __json_skipWhitespace(bytes, 0), 0)
  LET endIndex AS Integer = __json_skipWhitespace(bytes, parsed.index)
  IF endIndex <> len(bytes) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  RETURN parsed.value
END FUNC"#;

/// plan-120-E: the `parse(text, reviver)` overload. Parses with the untouched
/// 1-arg body, then walks the finished tree post-order — so a malformed
/// document fails before the reviver is ever called, exactly as in JavaScript.
#[rustfmt::skip]
const FUNC_BODY_REVIVE: &str =
r#"FUNC __json_parseRevive(value AS String, reviver AS FUNC(String, Json) AS Json) AS Json
  LET parsed AS Json = __json_parse(value)
  RETURN __json_revive("", parsed, reviver)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "parse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        // plan-120-E: two overloads, arity-selected (`datetime::parse`'s shape).
        expected_arguments: Some("String or String, FUNC(String, Json) AS Json"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![text_param()],
                return_type: ParameterType::named("Json"),
                // plan-120-A: the rendered Errors table is derived from this list,
                // and nothing cross-checks it against what the body raises — so it
                // has to be maintained by hand whenever a FAIL site changes code.
                // `ErrOverflow` is here because `__json_toNumber` re-raises
                // `toFloat`'s verdict rather than swallowing it; the grammar is
                // pre-validated, so overflow is the only verdict that gets through.
                errors: vec![
                    "ErrInvalidFormat",
                    "ErrInvalidSurrogate",
                    "ErrDepthExceeded",
                    "ErrOverflow",
                ],
                body: Body::mfb(FUNC_BODY, "__json_parse"),
            },
            // plan-120-E: the reviver form. Parses with the body above, then
            // walks the finished tree — so it raises exactly the same codes for
            // exactly the same documents, and the reviver only ever sees a tree
            // that parsed. An error the reviver itself raises propagates.
            Implementation {
                params: vec![
                    text_param(),
                    Parameter {
                        name: "reviver",
                        desc: "Called once for every parsed value, innermost first, with its key and the already-revived value; its return value is stored in place. The key is the member name inside an object, the index as a decimal string inside an array, and \"\" for the document root, which is called last.",
                        aliases: &[],
                        ty: ParameterType::func(
                            vec![ParameterType::String, ParameterType::named("Json")],
                            ParameterType::named("Json"),
                        ),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::named("Json"),
                errors: vec![
                    "ErrInvalidFormat",
                    "ErrInvalidSurrogate",
                    "ErrDepthExceeded",
                    "ErrOverflow",
                ],
                body: Body::mfb(FUNC_BODY_REVIVE, "__json_parseRevive"),
            },
        ],
    });
}

/// The `value` parameter, identical in both overloads.
fn text_param() -> Parameter {
    Parameter {
        name: "value",
        desc: "The JSON text to parse. Must contain exactly one complete JSON document, optionally surrounded by JSON whitespace. An empty or whitespace-only string is rejected.",
        aliases: &["text"],
        ty: ParameterType::String,
        default: DefaultValue::None,
    }
}
