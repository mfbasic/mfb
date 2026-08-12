//! `json::getOr` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__json_*` body lives here and replaces a
//! `'@@MFB_BODY:getOr@@` marker in package.mfb via assembled_source. Body
//! byte-significant (2-space indent → .ncode columns); do not reformat.

use crate::target::shared::registry::{BuiltinFunction, BuiltinOverload, ReturnType};

#[rustfmt::skip]
const BODY: &str =
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
      CASE ELSE
        RETURN defaultValue
    END MATCH
    current = nextValue
  NEXT
  RETURN current
END FUNC"#;

const OV: &[BuiltinOverload] = &[BuiltinOverload {
    params: super::P_GET_OR,
    return_type: ReturnType::Fixed("Json"),
}];

const INTRO: &str = r#"Read a nested `Json` value by key path, falling back to a default"#;
const DESC: &str = r#"`json::getOr` walks `path` through nested JSON objects exactly as `json::get`
does, but returns `defaultValue` instead of failing whenever traversal cannot
continue. Starting from `value`, each element of `path` is treated as an object
key: the current value must be a `JsonObj` that has that key, and the member
stored under it becomes the current value before the next element is applied.


Traversal stops and `defaultValue` is returned in exactly two situations: a path
element names a key that is absent from the current `JsonObj`, or traversal
reaches a `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, or `JsonArr` while path
elements remain. As with `json::get`, only object members are traversable —
array elements cannot be reached, so a path that descends into a `JsonArr`
returns the default.

An empty `path` performs no traversal and returns `value` unchanged, whatever
variant it is; `defaultValue` is never consulted in that case.

`defaultValue` is a `Json` value, not a sentinel, so the fallback is
indistinguishable from a value that was genuinely present. In particular
`json::getOr(doc, path, JsonNull[NOTHING])` returns the same thing whether the key
was absent or was present with the JSON value `null`. When that distinction
matters, use `json::get` and catch the failure instead.


The `value` and `defaultValue` arguments each accept the `Json` union or any one
of its six member types directly. `path` may also be passed by the name `key`,
and `defaultValue` under the names `default` or `fallback`."#;
const EX: &str = r#"Read a configuration flag with a fallback:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"config\":{}}")
  LET enabled AS json::Json = json::getOr(doc, ["config", "enabled"], JsonBool[FALSE])
  io::print(json::stringify(enabled))
END SUB
```

The default is also used when the path runs into a non-object:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"n\":3}")
  io::print(json::stringify(json::getOr(doc, ["n", "deeper"], JsonStr["absent"])))
END SUB
```

Pass the arguments by name:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"a\":{\"b\":1}}")
  io::print(json::stringify(json::getOr(doc, key := ["a", "b"], default := JsonNum[0.0])))
END SUB
```"#;

pub(crate) const GET_OR: BuiltinFunction =
    BuiltinFunction::mfb("json.getOr", "getOr", INTRO, DESC, &[], OV, BODY).with_example(EX);
