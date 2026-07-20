# parse

Parse a complete JSON document from text into a `Json` value

## Synopsis

```
json::parse(value AS String) AS Json
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

`json::parse` reads exactly one complete JSON document from `value` and returns
it as a `Json` union value. Leading and trailing JSON whitespace is skipped, and
anything other than whitespace after the first complete document is rejected — so
a string holding two documents, or a document followed by stray text, fails
rather than parsing the first and ignoring the rest.
[[src/builtins/json_package.mfb:__json_parse]]

Whitespace means exactly the four characters JSON allows: space, tab, carriage
return, and line feed. No other character is skippable, anywhere.
[[src/builtins/json_package.mfb:__json_isWhitespace]]

The input is scanned as a grapheme sequence, so the text is interpreted as
Unicode rather than bytes. Each JSON form maps to one variant of the `Json`
union: [[src/builtins/json_package.mfb:__json_parseValue]]

- `null` becomes `JsonNull[NOTHING]`.
- `true` and `false` become `JsonBool`.
- A number becomes `JsonNum`, holding a `Float`.
- A string becomes `JsonStr`.
- An array becomes `JsonArr`, holding a `List OF Json`; `[]` yields an empty list.
- An object becomes `JsonObj`, holding a `Map OF String TO Json`; `{}` yields an
  empty map. Duplicate keys collapse last-wins, because each pair is written into
  the map as it is read. [[src/builtins/json_package.mfb:__json_parseObjectItems]]

**Strings.** The escapes `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t`, and
`\uXXXX` are decoded. A `\u` escape must be exactly four hex digits; a high
surrogate must be followed immediately by `\u` and a low surrogate, and the pair
is combined into one code point. A lone low surrogate, an unpaired high
surrogate, an unknown escape letter, a truncated escape, or a code point outside
`0`–`1114111` is rejected. A raw control character (code point below `32`) that
appears unescaped inside a string is also rejected, as JSON requires.
[[src/builtins/json_package.mfb:__json_parseEscape]]
[[src/builtins/json_package.mfb:__json_parseUnicodeEscape]]
[[src/builtins/json_package.mfb:__json_parseString]]

**Numbers.** The token is validated against the JSON number grammar before it is
converted: an optional leading `-`, then either a single `0` or a nonzero digit
followed by further digits, then an optional `.` with at least one digit, then an
optional `e`/`E` with an optional sign and at least one digit. A leading `+`, a
leading `.`, a trailing `.`, a superfluous leading zero such as `01`, and the
JavaScript spellings `NaN` and `Infinity` are all rejected. The accepted token is
then converted to a `Float` (IEEE 754 binary64), so a value with more precision
or magnitude than binary64 can carry is approximated at parse time rather than
rejected. [[src/builtins/json_package.mfb:__json_validNumber]]
[[src/builtins/json_package.mfb:__json_toNumber]]

The parser is iterative rather than recursive at every scanning level — whitespace
runs, digit runs, string bodies, array items, and object members — so a long flat
document does not consume a native stack frame per character.
[[src/builtins/json_package.mfb:__json_skipWhitespace]]

The argument may also be passed by the name `text`.
[[src/builtins/json.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The JSON text to parse. Must contain exactly one complete JSON document, optionally surrounded by JSON whitespace. An empty or whitespace-only string is rejected. Also accepted under the name `text`. [[src/builtins/json.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Json` | The parsed document as a `Json` union value — one of `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, or `JsonObj`. [[src/builtins/json.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `value` is not one complete JSON document: it is empty or whitespace only, is truncated, carries non-whitespace content after the document, contains a malformed number, a bad or truncated escape, an unpaired or invalid surrogate, an out-of-range code point, an unescaped raw control character in a string, or any other syntax the grammar does not accept. [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] [[src/builtins/json_package.mfb:__json_parse]] |
| `77010001` | `ErrOutOfMemory` | The lists, maps, or strings that hold the parsed document cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Parse an object and read a nested value out of it:

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
```

## See also

- `mfb man json stringify`
- `mfb man json get`
- `mfb man json getOr`
- `mfb man json types`
- `mfb man json`
