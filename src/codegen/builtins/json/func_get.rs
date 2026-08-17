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

const INTRO: &str = r#"Read a nested `Json` value by following a path of object keys"#;

const DESC: &str = r#"`json::get` walks `path` through nested JSON objects and returns the value found
at the end. Starting from `value`, each element of `path` is treated as an object
key: the current value must be a `JsonObj`, and the member stored under that key
becomes the current value before the next element is applied. Traversal is left
to right, one key at a time.

Only object members are traversable. `JsonArr` has no keyed members, so array
elements cannot be reached with `json::get` at all — there is no numeric-index
form, and an index written as a string does not select an array element. Reaching
a `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, or `JsonArr` while path elements
remain fails, as does naming a key that is absent from the current `JsonObj`.
Both failures raise `ErrNotFound`.

An empty `path` performs no traversal and returns `value` unchanged, whatever
variant it is — including a non-object, since nothing needs to be traversed.

Both the failure cases are genuine failures, not sentinels: `json::get` never
returns a `JsonNull` to signal "missing", so it cannot be confused with a JSON
`null` that was really present in the document. When a missing key should produce
a fallback instead of failing, use `json::getOr`.

The first argument accepts the `Json` union or any one of its six member types
directly, so a `JsonObj` value can be passed without wrapping it. The second
argument may also be passed by the name `key`."#;

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
      CASE ELSE
        FAIL error(77050004, "Requested item, key, file, or resource was not found.")
    END MATCH
    current = nextValue
  NEXT
  RETURN current
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
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
                    desc: "The value to read from. Accepts the Json union or any of JsonNull, JsonBool, JsonNum, JsonStr, JsonArr, JsonObj; traversal only succeeds through JsonObj members.",
                    aliases: &[],
                    ty: ParameterType::Named("Json"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "path",
                    desc: "The object keys to follow, from the root inward. Each element selects a member by exact String key. An empty list selects value itself.",
                    aliases: &["key"],
                    ty: ParameterType::list_of(ParameterType::String),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Named("Json"),
            errors: vec!["ErrNotFound"],
            body: Body::mfb(FUNC_BODY, "__json_get"),
        }],
    });
}
