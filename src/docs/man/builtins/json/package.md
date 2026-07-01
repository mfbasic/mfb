# json

Parse, build, serialize, and read JSON values as a `Json` tree

## Synopsis

```
IMPORT json
json::parse(text)
json::stringify(value)
json::get(value, path)
json::getOr(value, path, default)
```

## Description

The `json` package converts between JSON text and a `Json` value tree and reads
members out of that tree. `json::parse` turns a UTF-8 `String` holding one
complete JSON document into a `Json` value, `json::stringify` renders a `Json`
value back into compact JSON text, and `json::get` and `json::getOr` walk a path
of object keys to a nested member. `json` is a built-in package written in
MFBASIC source over the `collections`, `strings`, and `encoding` packages, so
`IMPORT json` needs no manifest dependency. [[src/builtins/json.rs:augmented_project]]

The package defines the `Json` union and its six member types. `Json` is a
`UNION` over `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, and
`JsonObj`, each a record wrapping one field: `JsonNull` holds `Nothing`,
`JsonBool` holds a `Boolean`, `JsonNum` holds a `Float`, `JsonStr` holds a
`String`, `JsonArr` holds a `List OF Json`, and `JsonObj` holds a
`Map OF String TO Json`. Every JSON form maps to exactly one variant, and
`json::stringify` accepts either the `Json` union or any one of its member types
directly. Because numbers are carried as `Float`, very large or very precise
values may lose precision in a parse/stringify round trip, and a `JsonNum`
holding a non-finite `Float` (NaN or infinity) has no JSON form. [[src/builtins/json.rs:is_builtin_type]]

Serialization is compact: `json::stringify` emits no insignificant whitespace,
preserves array item order, emits object pairs in the map's iteration order, and
applies the standard JSON string escapes. Parsing reads one complete document,
allows surrounding JSON whitespace, and rejects any trailing non-whitespace
content. [[src/builtins/json_package.mfb:__json_parse]]

The path readers operate only on object members. `json::get` and `json::getOr`
follow a `List OF String` of object keys left to right from `value`, requiring a
`JsonObj` at each step; an empty path returns `value` unchanged. They do not copy
`value`: the located `Json` value is returned directly. `json::get` fails when a
key is missing or the current value is not an object, whereas `json::getOr`
returns its default value in those cases instead of failing. [[src/builtins/json_package.mfb:__json_getOr]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | raised by `parse` when the text is not a single valid JSON document — empty, truncated, trailing non-whitespace content, a bad escape or surrogate, an out-of-range code point, a raw control character in a string, or a malformed number — and by `stringify` when a `JsonNum` holds a non-finite `Float` (NaN or positive or negative infinity) [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050004` | `ErrNotFound` | raised by `get` when a path element names a key absent from the current `JsonObj`, or when traversal reaches a non-object `Json` value while path elements remain [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |
