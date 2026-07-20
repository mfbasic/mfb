# types

The `json` package record types and the `Json` union over them

## Synopsis

```
json::JsonNull
json::JsonBool
json::JsonNum
json::JsonStr
json::JsonArr
json::JsonObj
json::Json
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

The `json` package represents a JSON value as a tree of six single-field record
types, plus the `Json` union that ranges over all six. Every JSON document parses
to exactly one of the six variants, and every `json::` function that takes or
returns "a JSON value" is typed in terms of `Json`.
[[src/builtins/json_package.mfb:Json]]

`JsonNull`, `JsonBool`, `JsonNum`, and `JsonStr` are leaf scalars. `JsonArr` and
`JsonObj` are containers whose single field holds further `Json` values, which is
what lets a document nest to arbitrary depth.
[[src/builtins/json_package.mfb:JsonArr]]

All seven names are builtin types, referenced bare rather than package-qualified:
write `JsonStr["x"]`, not `json::JsonStr["x"]`. The union type itself is written
`json::Json` in a declaration such as `LET doc AS json::Json`.
[[src/builtins/json.rs:is_builtin_type]]

Anywhere a `Json` is accepted, a member type is accepted directly as well —
`json::stringify`, `json::get`, and the `value`/`defaultValue` arguments of
`json::getOr` all take the union or any one of the six records, so a scalar does
not have to be widened by hand. [[src/builtins/json.rs:is_json_value_type]]

To take a value apart, `MATCH` on it and bind the variant, which is how the
package's own code reads a tree. `json::get` and `json::getOr` are the shortcut
for the common case of descending through `JsonObj` members by key.
[[src/builtins/json_package.mfb:__json_stringify]]

## Types

### json::JsonNull

The JSON literal `null`. [[src/builtins/json_package.mfb:JsonNull]]

| Field | Type | Description |
| --- | --- | --- |
| `value` | `Nothing` | Always the absent value `NOTHING`. The record carries no data and exists only to mark the null form; construct it as `JsonNull[NOTHING]`. |

### json::JsonBool

A JSON boolean. [[src/builtins/json_package.mfb:JsonBool]]

| Field | Type | Description |
| --- | --- | --- |
| `value` | `Boolean` | `TRUE` for the JSON literal `true`, `FALSE` for `false`. |

### json::JsonNum

A JSON number. [[src/builtins/json_package.mfb:JsonNum]]

| Field | Type | Description |
| --- | --- | --- |
| `value` | `Float` | The numeric payload as an IEEE 754 binary64. JSON has one number type, so integers and fractions alike land here; a literal with more precision or magnitude than binary64 can hold is approximated at parse time. `json::stringify` emits the shortest decimal that reads back to the same `Float`, so a parse/stringify round trip preserves the value exactly. [[src/builtins/json_package.mfb:__json_stringifyNumber]] |

### json::JsonStr

A JSON string. [[src/builtins/json_package.mfb:JsonStr]]

| Field | Type | Description |
| --- | --- | --- |
| `value` | `String` | The decoded contents, with JSON escapes already resolved — the field holds the real characters, not the escaped source text. `json::stringify` reapplies escaping on the way out. [[src/builtins/json_package.mfb:__json_escapeString]] |

### json::JsonArr

A JSON array. [[src/builtins/json_package.mfb:JsonArr]]

| Field | Type | Description |
| --- | --- | --- |
| `items` | `List OF Json` | The elements in document order, each a nested `Json`. Order is preserved by `json::stringify`. Read them with the ordinary `collections::` accessors — `json::get` cannot index into an array. An empty array yields an empty list. |

### json::JsonObj

A JSON object. [[src/builtins/json_package.mfb:JsonObj]]

| Field | Type | Description |
| --- | --- | --- |
| `fields` | `Map OF String TO Json` | The members keyed by name, each value a nested `Json`. Duplicate keys in the source collapse last-wins during parsing. This is the map that `json::get` and `json::getOr` descend through. `json::stringify` emits the pairs in the map's iteration order, which is not guaranteed to match document order. [[src/builtins/json_package.mfb:__json_parseObjectItems]] |

### json::Json

The union over all six record types. Every parsed JSON value is one of them, and
this is the type to declare when a value's form is not known statically.
[[src/builtins/json_package.mfb:Json]]

## Examples

Dispatch on the variant of a parsed value:

```
IMPORT json
IMPORT io

SUB describe(value AS json::Json)
  MATCH value
    CASE JsonNull(n)
      io::print("null")
    CASE JsonBool(b)
      io::print("bool " & toString(b.value))
    CASE JsonNum(n)
      io::print("number " & toString(n.value))
    CASE JsonStr(s)
      io::print("string " & s.value)
    CASE JsonArr(a)
      io::print("array of " & toString(len(a.items)))
    CASE JsonObj(o)
      io::print("object of " & toString(len(o.fields)))
  END MATCH
END SUB

SUB main
  describe(json::parse("{\"a\":1}"))
  describe(json::parse("[1,2,3]"))
END SUB
```

Build a document from the record types and serialize it. Note that a member value
has to be widened to `json::Json` by a typed binding before it goes into a
`Map OF String TO json::Json` — the widening that call arguments get is not
applied to a collection element:

```
IMPORT json
IMPORT io
IMPORT collections

SUB main
  LET name AS json::Json = JsonStr["Ada"]
  LET age AS json::Json = JsonNum[36.0]
  MUT fields AS Map OF String TO json::Json = Map OF String TO json::Json {}
  fields = collections::set(fields, "name", name)
  fields = collections::set(fields, "age", age)
  io::print(json::stringify(JsonObj[fields]))
END SUB
```

## See also

- `mfb man json`
- `mfb man json parse`
- `mfb man json stringify`
- `mfb man json get`
- `mfb man json getOr`
