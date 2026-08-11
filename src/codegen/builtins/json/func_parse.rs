//! `json::parse` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:parse@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __json_parse(value AS String) AS Json
  LET chars AS List OF String = strings::graphemes(value)
  ' bug-422: depth 0 seeds the structural nesting-depth guard threaded through
  ' the value/array/object parsers below.
  LET parsed AS __json_Node = __json_parseValue(chars, __json_skipWhitespace(chars, 0), 0)
  LET endIndex AS Integer = __json_skipWhitespace(chars, parsed.index)
  IF endIndex <> len(chars) THEN
    FAIL error(77050003, "invalid JSON format")
  END IF
  RETURN parsed.value
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_PARSE,
    return_type: ReturnType::Fixed("Json"),
}];

const INTRO: &str = r#"Parse a complete JSON document from text into a `Json` value"#;
const DESC: &str = r#"`json::parse` reads exactly one complete JSON document from `value` and returns
it as a `Json` union value. Leading and trailing JSON whitespace is skipped, and
anything other than whitespace after the first complete document is rejected — so
a string holding two documents, or a document followed by stray text, fails
rather than parsing the first and ignoring the rest.


Whitespace means exactly the four characters JSON allows: space, tab, carriage
return, and line feed. No other character is skippable, anywhere.


The input is scanned as a grapheme sequence, so the text is interpreted as
Unicode rather than bytes. Each JSON form maps to one variant of the `Json`
union:

- `null` becomes `JsonNull[NOTHING]`.
- `true` and `false` become `JsonBool`.
- A number becomes `JsonNum`, holding a `Float`.
- A string becomes `JsonStr`.
- An array becomes `JsonArr`, holding a `List OF Json`; `[]` yields an empty list.
- An object becomes `JsonObj`, holding a `Map OF String TO Json`; `{}` yields an
  empty map. Duplicate keys collapse last-wins, because each pair is written into
  the map as it is read.

**Strings.** The escapes `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t`, and
`\uXXXX` are decoded. A `\u` escape must be exactly four hex digits; a high
surrogate must be followed immediately by `\u` and a low surrogate, and the pair
is combined into one code point. A lone low surrogate, an unpaired high
surrogate, an unknown escape letter, a truncated escape, or a code point outside
`0`–`1114111` is rejected. A raw control character (code point below `32`) that
appears unescaped inside a string is also rejected, as JSON requires.




**Numbers.** The token is validated against the JSON number grammar before it is
converted: an optional leading `-`, then either a single `0` or a nonzero digit
followed by further digits, then an optional `.` with at least one digit, then an
optional `e`/`E` with an optional sign and at least one digit. A leading `+`, a
leading `.`, a trailing `.`, a superfluous leading zero such as `01`, and the
JavaScript spellings `NaN` and `Infinity` are all rejected. The accepted token is
then converted to a `Float` (IEEE 754 binary64), so a value with more precision
or magnitude than binary64 can carry is approximated at parse time rather than
rejected.


The parser is iterative rather than recursive at every *scanning* level —
whitespace runs, digit runs, string bodies, and the sibling elements of a single
array or object — so a long flat document does not consume a native stack frame
per character.

Structural *nesting*, by contrast, does descend one call per level, so it is
bounded: a document nested beyond a fixed structural depth (256 levels of arrays
and objects combined) is rejected with `77050003` rather than being allowed to
exhaust the native stack. Any realistic document is far within this limit; the cap
exists only so that adversarial deeply-nested input fails cleanly instead of
crashing the process.

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
    RETURN JsonNull[NOTHING]
  END TRAP
END FUNC
```"#;

pub(crate) const PARSE: BuiltinFunction =
    BuiltinFunction::mfb("json.parse", "parse", INTRO, DESC, &[], OV, BODY).with_example(EX);
