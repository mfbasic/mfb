# stringify

Serialize a `Json` value as compact JSON text

## Synopsis

```
json::stringify(value AS Json) AS String
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

`json::stringify` serializes `value` into a single JSON document with no
indentation, no line breaks, and no whitespace between tokens. Arrays and objects
are serialized recursively, so one call renders a whole tree.
[[src/builtins/json_package.mfb:__json_stringify]]

Each variant maps to its JSON form: `JsonNull` emits `null`; `JsonBool` emits
`true` or `false`; `JsonStr` emits a double-quoted escaped string; `JsonArr`
emits `[`, its items in list order separated by `,`, and `]`; `JsonObj` emits
`{`, its members as `"key":value` separated by `,`, and `}`, with each key
escaped as a JSON string. An empty `JsonArr` emits `[]` and an empty `JsonObj`
emits `{}`. Object members are emitted in the map's iteration order, which is not
guaranteed to be insertion order or sorted order — do not rely on it for stable
output or for byte-comparison of two documents.
[[src/builtins/json_package.mfb:__json_stringify]]

**String escaping.** `"` and `\` are escaped, and so is `/` — every forward slash
is emitted as `\/`. That is valid JSON and parses back identically, but it means
`json::stringify` output is not byte-identical to what most other JSON writers
produce. The C0 escapes `\b`, `\t`, `\n`, `\f`, and `\r` are used where they
apply, and any remaining control character below code point `32` is emitted as a
`\u00XX` escape. Everything else, including all non-ASCII text, is emitted
literally as UTF-8 rather than as `\u` escapes.
[[src/builtins/json_package.mfb:__json_escapeString]]
[[src/builtins/json_package.mfb:__json_controlEscape]]

**Numbers.** A `JsonNum` holds a `Float`, and the rendering is chosen so that it
round-trips: the whole-number form is tried first, so an integral value emits as
`100` rather than `100.0`, and otherwise the *shortest* fractional rendering that
parses back to exactly the same `Float` is searched for and used. The round trip
is verified rather than assumed — `3.141592653589793` serializes with all of its
digits intact, not truncated to a fixed precision. If no rendering round-trips,
the call fails rather than emitting a silently lossy number.
[[src/builtins/json_package.mfb:__json_stringifyNumber]]

The number path also guards against a non-finite `Float`, but that guard is
unreachable from ordinary MFBASIC code: no user-accessible `Float` is non-finite,
because storing a `NaN` or an infinity into a record field is an observation
boundary that fails first with `ErrFloatNaN` or `ErrFloatOverflow`. A `JsonNum`
therefore cannot be constructed around a non-finite value in the first place.
[[src/builtins/json_package.mfb:__json_isInvalidNumberText]]

The argument accepts the `Json` union or any one of its six member types
(`JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, `JsonObj`) directly, so
a scalar member value can be serialized without wrapping it.
[[src/builtins/json.rs:is_json_value_type]]

The output is always re-readable by `json::parse`, which makes
`parse`/`stringify` a lossless round trip for every value `parse` can produce.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Json` | The value to serialize. Accepts the `Json` union or any of `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, `JsonObj`. [[src/builtins/json.rs:call_param_names]] [[src/builtins/json.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The compact JSON text for `value`, containing no insignificant whitespace and readable back with `json::parse`. [[src/builtins/json.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | No decimal rendering of a `JsonNum`'s `Float` parses back to the same value, or that `Float` is non-finite. Both indicate a formatter fault rather than bad input, and neither is reachable from a `Json` value built by ordinary MFBASIC code. [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] [[src/builtins/json_package.mfb:__json_stringifyNumber]] |
| `77010001` | `ErrOutOfMemory` | The result string or an intermediate fragment cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Round-trip a document through text:

```
IMPORT json
IMPORT io

SUB main
  LET doc AS json::Json = json::parse("{\"name\":\"Ada\",\"n\":3}")
  io::print(json::stringify(doc))
END SUB
```

Serialize a member type directly, without wrapping it in the union:

```
IMPORT json
IMPORT io

SUB main
  io::print(json::stringify(JsonBool[TRUE]))
  io::print(json::stringify(JsonStr["a/b"]))
END SUB
```

Build a value and serialize it:

```
IMPORT json
IMPORT io

SUB main
  LET items AS List OF json::Json = [JsonNum[1.0], JsonNum[2.5]]
  io::print(json::stringify(JsonArr[items]))
END SUB
```

## See also

- `mfb man json parse`
- `mfb man json get`
- `mfb man json getOr`
- `mfb man json types`
- `mfb man json`
