//! `json::get` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:get@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Read a nested `json::Json` value by following a path of object keys and array indexes"#;

const DESC: &str = r#"`json::get` walks `path` through a nested JSON tree and returns the value found
at the end. Starting from `value`, each element of `path` selects one member of
the current value, which becomes the current value before the next element is
applied. Traversal is left to right, one step at a time.

**What a path element means depends on what it lands on.** On a `json::JsonObj`
it is an object key, matched exactly. On a `json::JsonArr` it is a zero-based
array index, written in decimal — so `["items", "1"]` reads the second element of
the `items` array.

A token counts as an index only if it is spelled the way RFC 6901 spells one:
a single `0`, or a nonzero digit followed by further digits. `"01"`,
`"+1"`, `"-1"`, `"1 "` and `""` are not indexes, and neither is a run of more than
18 digits (no list is that long). On an array, such a token simply finds nothing.

Because the meaning is decided by the variant underfoot and never by the token
itself, `"1"` remains an ordinary key when the current value is an object, even if
the same document has arrays elsewhere.

Reaching a `json::JsonNull`, `json::JsonBool`, `json::JsonNum` or `json::JsonStr`
while path elements remain fails, as does naming a key absent from the current
`json::JsonObj`, or an index outside the current `json::JsonArr`. All of these
raise `ErrNotFound`.

An empty `path` performs no traversal and returns `value` unchanged, whatever
variant it is — including a non-object, since nothing needs to be traversed.

Both the failure cases are genuine failures, not sentinels: `json::get` never
returns a `json::JsonNull` to signal "missing", so it cannot be confused with a JSON
`null` that was really present in the document. When a missing key should produce
a fallback instead of failing, use `json::getOr`.

The first argument accepts the `json::Json` union or any one of its six member types
directly, so a `json::JsonObj` value can be passed without wrapping it. The second
argument may also be passed by the name `key`.

Unlike JavaScript's `JSON.parse`, which hands back a plain object you then index
with ordinary syntax, `json::get` is how a whole path is read in one call — there
is no separate array accessor to reach for."#;

const EX: &str = r#"Read a nested member by key path:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"user\":{\"address\":{\"city\":\"Oslo\"}}}")
  LET city AS json::Json = json::get(doc, ["user", "address", "city"])
  io::print(json::stringify(city))
END SUB
```

Step into an array by writing its index as a decimal string, mixing keys and
indexes freely along one path:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"items\":[{\"name\":\"a\"},{\"name\":\"b\"}]}")
  io::print(json::stringify(json::get(doc, ["items", "1", "name"])))
  io::print(json::stringify(json::get(doc, ["items", "0"])))
END SUB
```

prints:

```
"b"
{"name":"a"}
```

An empty path returns the root unchanged. The empty list needs a typed binding,
because a bare `[]` literal has no element type of its own:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("[1,2,3]")
  LET here AS List OF String = []
  io::print(json::stringify(json::get(doc, here)))
END SUB
```

Turn a missing key into a caught failure:

```
IMPORT json
IMPORT io

SUB show(doc AS json::Json)
  io::print(json::stringify(json::get(doc, ["config", "enabled"])))
  EXIT SUB
  TRAP(e)
    io::print("absent: " & toString(e.code))
    EXIT SUB
  END TRAP
END SUB

SUB main
  show(json::parse("{\"config\": {\"enabled\": true}}"))
  show(json::parse("{}"))
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __json_get(value AS Json, path AS List OF String) AS Json
  MUT current AS Json = value
  FOR EACH key IN path
    MUT nextValue AS Json = current
    LET currentValue AS Json = current
    MATCH currentValue
      CASE JsonObj(obj)
        nextValue = collections::get(obj.fields, key)
      CASE JsonArr(arr)
        ' plan-120-B: on an ARRAY the same token is read as a decimal index
        ' (RFC 6901). Which meaning a token has depends only on the variant
        ' underfoot, so a key that looks like a number is still a key on an
        ' object -- no existing program changes behavior, because reaching an
        ' array used to fail unconditionally.
        LET index AS Integer = __json_arrayIndex(key)
        IF index < 0 OR index >= len(arr.items) THEN
          FAIL error(77050004, "Requested item, key, file, or resource was not found.")
        END IF
        nextValue = collections::get(arr.items, index)
      CASE ELSE
        FAIL error(77050004, "Requested item, key, file, or resource was not found.")
    END MATCH
    current = nextValue
  NEXT
  RETURN current
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "get",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The value to read from. Accepts the Json union or any of JsonNull, JsonBool, JsonNum, JsonStr, JsonArr, JsonObj; traversal steps through JsonObj members and JsonArr elements.",
                    aliases: &[],
                    ty: ParameterType::named("Json"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "path",
                    desc: "The steps to follow, from the root inward. On an object a step is an exact String key; on an array it is a zero-based decimal index (RFC 6901 spelling: 0, or a nonzero digit then digits). An empty list selects value itself.",
                    aliases: &["key"],
                    ty: ParameterType::list_of(ParameterType::String),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named("Json"),
            errors: vec!["ErrNotFound"],
            body: Body::mfb(FUNC_BODY, "__json_get"),
        }],
    });
}
