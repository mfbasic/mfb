# getOr

Read a nested `Json` value by key path, falling back to a default

## Synopsis

```
json::getOr(value AS Json, path AS List OF String, defaultValue AS Json) AS Json
```

## Package

`json`

## Imports

```
IMPORT json
```

`json` is a built-in package, so `IMPORT json` needs no manifest dependency.
[[src/builtins/json.rs:augmented_project]]

## Description

`json::getOr` walks `path` through nested JSON objects exactly as `json::get`
does, but returns `defaultValue` instead of failing whenever traversal cannot
continue. Starting from `value`, each element of `path` is treated as an object
key: the current value must be a `JsonObj` that has that key, and the member
stored under it becomes the current value before the next element is applied.
[[src/builtins/json_package.mfb:__json_getOr]]

Traversal stops and `defaultValue` is returned in exactly two situations: a path
element names a key that is absent from the current `JsonObj`, or traversal
reaches a `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, or `JsonArr` while path
elements remain. As with `json::get`, only object members are traversable —
array elements cannot be reached, so a path that descends into a `JsonArr`
returns the default. [[src/builtins/json_package.mfb:__json_getOr]]

An empty `path` performs no traversal and returns `value` unchanged, whatever
variant it is; `defaultValue` is never consulted in that case.

`defaultValue` is a `Json` value, not a sentinel, so the fallback is
indistinguishable from a value that was genuinely present. In particular
`json::getOr(doc, path, JsonNull[NOTHING])` returns the same thing whether the key
was absent or was present with the JSON value `null`. When that distinction
matters, use `json::get` and catch the failure instead.
[[src/builtins/json_package.mfb:__json_get]]

The `value` and `defaultValue` arguments each accept the `Json` union or any one
of its six member types directly. `path` may also be passed by the name `key`,
and `defaultValue` under the names `default` or `fallback`.
[[src/builtins/json.rs:is_json_value_type]]
[[src/builtins/json.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Json` | The value to read from. Accepts the `Json` union or any of `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, `JsonObj`; traversal only succeeds through `JsonObj` members. [[src/builtins/json.rs:call_param_names]] [[src/builtins/json.rs:resolve_call]] |
| `path` | `List OF String` | The object keys to follow, from the root inward. Each element selects a member by exact `String` key. An empty list selects `value` itself. Also accepted under the name `key`. [[src/builtins/json.rs:call_param_names]] |
| `defaultValue` | `Json` | Returned when traversal cannot continue. Accepts the `Json` union or any member type. Also accepted under the names `default` and `fallback`. [[src/builtins/json.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Json` | The value reached by following every key in `path`; `value` unchanged when `path` is empty; `defaultValue` when a key is missing or a non-object is reached with keys remaining. [[src/builtins/json.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Read a configuration flag with a fallback:

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
```

## See also

- `mfb man json get`
- `mfb man json parse`
- `mfb man json stringify`
- `mfb man json types`
- `mfb man json`
