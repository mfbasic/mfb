//! `json::getOr` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:getOr@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Read a nested `json::Json` value by path, falling back to a default"#;

const DESC: &str = r#"`json::getOr` walks `path` through a nested JSON tree exactly as `json::get`
does, but returns `defaultValue` instead of failing whenever traversal cannot
continue. Starting from `value`, each element of `path` selects one member of the
current value, which becomes the current value before the next element is applied.

As in `json::get`, what a path element means depends on what it lands on: an
object key on a `json::JsonObj`, and a zero-based decimal array index on a
`json::JsonArr`, spelled the RFC 6901 way (a single `0`, or a nonzero digit
followed by further digits — never `"01"`, `"+1"`, `"-1"` or `"1 "`).

Traversal stops and `defaultValue` is returned whenever the current step finds
nothing: a key absent from the current `json::JsonObj`, an index outside the
current `json::JsonArr`, a token on an array that is not spelled as an index, or
a `json::JsonNull`, `json::JsonBool`, `json::JsonNum` or `json::JsonStr` reached
while path elements remain.

`json::getOr` never fails, whatever `path` contains.

An empty `path` performs no traversal and returns `value` unchanged, whatever
variant it is; `defaultValue` is never consulted in that case.

`defaultValue` is a `json::Json` value, not a sentinel, so the fallback is
indistinguishable from a value that was genuinely present. In particular
`json::getOr(doc, path, json::JsonNull[NOTHING])` returns the same thing whether the key
was absent or was present with the JSON value `null`. When that distinction
matters, use `json::get` and catch the failure instead.

The `value` and `defaultValue` arguments each accept the `json::Json` union or any one
of its six member types directly. `path` may also be passed by the name `key`,
and `defaultValue` under the names `default` or `fallback`."#;

const EX: &str = r#"Read a configuration flag with a fallback:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"config\":{}}")
  LET enabled AS json::Json = json::getOr(doc, ["config", "enabled"], json::JsonBool[FALSE])
  io::print(json::stringify(enabled))
END SUB
```

The default is also used when the path runs into a non-object:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"n\":3}")
  io::print(json::stringify(json::getOr(doc, ["n", "deeper"], json::JsonStr["absent"])))
END SUB
```

An array index that is out of range, or a token that is not spelled as an index,
returns the default rather than failing:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"items\":[10,20]}")
  LET miss AS json::Json = json::JsonStr["none"]
  io::print(json::stringify(json::getOr(doc, ["items", "1"], miss)))
  io::print(json::stringify(json::getOr(doc, ["items", "9"], miss)))
  io::print(json::stringify(json::getOr(doc, ["items", "01"], miss)))
END SUB
```

prints:

```
20
"none"
"none"
```

Pass the arguments by name:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"a\":{\"b\":1}}")
  io::print(json::stringify(json::getOr(doc, key := ["a", "b"], default := json::JsonNum[0.0])))
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __json_getOr(value AS Json, path AS List OF String, defaultValue AS Json) AS Json
  MUT current AS Json = value
  FOR EACH key IN path
    MUT nextValue AS Json = current
    LET currentValue AS Json = current
    MATCH currentValue
      CASE JsonObj(obj)
        IF collections::hasKey(obj.fields, key) THEN
          nextValue = collections::get(obj.fields, key)
        ELSE
          RETURN defaultValue
        END IF
      CASE JsonArr(arr)
        ' plan-120-B: the `json::get` arm, with the FAIL replaced by the default
        ' -- these two bodies are one contract written twice and must stay
        ' structurally parallel. `__json_arrayIndex` never fails, so getOr stays
        ' total for every token including an overlong digit run.
        LET index AS Integer = __json_arrayIndex(key)
        IF index < 0 OR index >= len(arr.items) THEN
          RETURN defaultValue
        END IF
        nextValue = collections::get(arr.items, index)
      CASE ELSE
        RETURN defaultValue
    END MATCH
    current = nextValue
  NEXT
  RETURN current
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getOr",
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
                Parameter {
                    name: "default",
                    desc: "Returned when traversal cannot continue. Accepts the Json union or any member type.",
                    aliases: &["defaultValue", "fallback"],
                    ty: ParameterType::named("Json"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named("Json"),
            errors: vec![],
            body: Body::mfb(FUNC_BODY, "__json_getOr"),
        }],
    });
}
