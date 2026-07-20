# get

Read a nested `Json` value by following a path of object keys

## Synopsis

```
json::get(value AS Json, path AS List OF String) AS Json
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

`json::get` walks `path` through nested JSON objects and returns the value found
at the end. Starting from `value`, each element of `path` is treated as an object
key: the current value must be a `JsonObj`, and the member stored under that key
becomes the current value before the next element is applied. Traversal is left
to right, one key at a time.
[[src/builtins/json_package.mfb:__json_get]]

Only object members are traversable. `JsonArr` has no keyed members, so array
elements cannot be reached with `json::get` at all — there is no numeric-index
form, and an index written as a string does not select an array element. Reaching
a `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, or `JsonArr` while path elements
remain fails, as does naming a key that is absent from the current `JsonObj`.
Both failures raise `ErrNotFound`. [[src/builtins/json_package.mfb:__json_get]]

An empty `path` performs no traversal and returns `value` unchanged, whatever
variant it is — including a non-object, since nothing needs to be traversed.

Both the failure cases are genuine failures, not sentinels: `json::get` never
returns a `JsonNull` to signal "missing", so it cannot be confused with a JSON
`null` that was really present in the document. When a missing key should produce
a fallback instead of failing, use `json::getOr`.
[[src/builtins/json_package.mfb:__json_getOr]]

The first argument accepts the `Json` union or any one of its six member types
directly, so a `JsonObj` value can be passed without wrapping it. The second
argument may also be passed by the name `key`.
[[src/builtins/json.rs:is_json_value_type]]
[[src/builtins/json.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Json` | The value to read from. Accepts the `Json` union or any of `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, `JsonObj`; traversal only succeeds through `JsonObj` members. [[src/builtins/json.rs:call_param_names]] [[src/builtins/json.rs:resolve_call]] |
| `path` | `List OF String` | The object keys to follow, from the root inward. Each element selects a member by exact `String` key — no wildcards, globbing, or array indices. An empty list selects `value` itself. Also accepted under the name `key`. [[src/builtins/json.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Json` | The value reached by following every key in `path`. With an empty `path`, `value` unchanged. [[src/builtins/json.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050004` | `ErrNotFound` | A path element names a key that is absent from the current `JsonObj`, or traversal reaches a non-object value (`JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, or `JsonArr`) while path elements remain. [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] [[src/builtins/json_package.mfb:__json_get]] |

## Examples

Read a nested member by key path:

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
```

## See also

- `mfb man json getOr`
- `mfb man json parse`
- `mfb man json stringify`
- `mfb man json types`
- `mfb man json`
