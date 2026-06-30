# JSON Data Model

The `json::` package is implemented as injected MFBASIC source, not native Rust.
It defines the `Json` union — a closed, recursive sum type that mirrors the six
JSON value kinds — plus a hand-written recursive-descent parser, a deterministic
stringifier, and path-based accessors. This topic specifies the data model and
the parse/stringify/access algorithms a faithful reimplementation must reproduce.

The per-function `json::` API (signatures, parameters, return types, error codes)
is owned by `./mfb man json`; this topic specifies only the *model and behavior*
behind that API.

## The `Json` union

`Json` is an exported `UNION` of six exported single-field record types, one per
JSON value kind. Each variant wraps its payload in a named record rather than
storing it bare, so a `Json` is always a tagged record value.
[[src/builtins/json_package.mfb:Json]]

| Variant | Record field | MFBASIC type | Represents |
| --- | --- | --- | --- |
| `JsonNull` | `value AS Nothing` | `Nothing` | `null` |
| `JsonBool` | `value AS Boolean` | `Boolean` | `true` / `false` |
| `JsonNum` | `value AS Float` | `Float` | any number |
| `JsonStr` | `value AS String` | `String` | string |
| `JsonArr` | `items AS List OF Json` | `List OF Json` | array |
| `JsonObj` | `fields AS Map OF String TO Json` | `Map OF String TO Json` | object |

The recursion is in `JsonArr.items` and `JsonObj.fields`, both of which hold
`Json` values, making the model an arbitrarily deep tree. Objects are backed by
the standard `collections::` `Map OF String TO Json`; arrays by `List OF Json`.
There is no separate integer kind — see below.

A literal is constructed with the record-literal form, e.g. `JsonNull[NOTHING]`,
`JsonBool[TRUE]`, `JsonStr[parsed.value]`, `JsonNum[numberValue]`,
`JsonArr[items]`, `JsonObj[fields]`. Consumers discriminate with `MATCH` over the
union, binding the wrapped record (`CASE JsonObj(obj)` then `obj.fields`).
[[src/builtins/json_package.mfb:__json_stringify]]

### Numbers are always `Float`

JSON has a single numeric type and the model follows: every number — integral or
fractional — is stored in `JsonNum.value` as a 64-bit `Float`. There is no
`JsonInt`. Parsing converts the lexed numeric token to `Float` via `toFloat`;
out-of-range or unparseable tokens fail (see grammar).
[[src/builtins/json_package.mfb:__json_parseNumber]]

## AST injection (front-end seam)

`json::` is not linked as a precompiled object. The Rust seam in
`src/builtins/json.rs` carries the package source as `include_str!` and, when the
program imports `json`, appends the parsed package file into the project AST
before the rest of the front end runs. `augmented_project` clones the project and
pushes `source_file()` (the parsed `json_package.mfb`) only if `uses_package`
finds an `IMPORT json`; otherwise the project is returned unchanged. The package
source then flows through the same resolver / monomorphization / codegen path as
user code. [[src/builtins/json.rs:augmented_project]]

The seam also models the four public calls (`json.parse`, `json.stringify`,
`json.get`, `json.getOr`) for type resolution: `resolve_call` maps an exact
argument-type signature to a return type, and `implementation_name` rewrites each
public call to its `__json_*` source FUNC. The `Json*` family is registered as
built-in types, and `is_json_value_type` treats `Json` and all six variant record
names as acceptable wherever a `Json` argument is expected (so a bare `JsonObj`
may be passed where `Json` is wanted). [[src/builtins/json.rs:resolve_call]]
[[src/builtins/json.rs:is_json_value_type]]

See `./mfb spec architecture frontend` for the injection ordering and
`./mfb spec architecture monomorphization` for how the generic `List OF Json` /
`Map OF String TO Json` instantiations are produced.

## Parse acceptance grammar

`__json_parse` graphemizes the input, skips leading whitespace, parses one value,
skips trailing whitespace, and requires the cursor to be exactly at end-of-input;
any trailing non-whitespace fails. All failures raise error `77050003`
("invalid JSON format"). [[src/builtins/json_package.mfb:__json_parse]]

The accepted grammar (RFC-8259-aligned, with the noted deviations):

```
value      := ws val ws
val        := "null" | "true" | "false" | string | number | array | object
array      := "[" ws "]" | "[" ws value ("," value)* "]"
object     := "{" ws "}" | "{" ws member ("," member)* "}"
member     := ws string ws ":" value
string     := '"' char* '"'
char       := unescaped | "\" escape
escape     := '"' | "\" | "/" | "b" | "f" | "n" | "r" | "t" | "u" hex hex hex hex
number     := "-"? int frac? exp?
int        := "0" | nonzero digit*
frac       := "." digit+
exp        := ("e" | "E") ("+" | "-")? digit+
ws         := (" " | "\t" | "\n" | "\r")*
```

Dispatch is by first non-whitespace character: `n`/`t`/`f` route to literal
matching (`__json_expectLiteral`), `"` to string, `[`/`{` to array/object, and
everything else to the number lexer. [[src/builtins/json_package.mfb:__json_parse]]

Notable parse rules and deviations:

- **Numbers**: a number token is collected greedily up to the next `,`, `]`, `}`,
  or whitespace, then validated by `__json_validNumber` against the grammar above
  *before* `toFloat` conversion. The exponent marker accepts both `e` and `E`; a
  leading `0` may not be followed by more integer digits; a fraction requires at
  least one digit after `.`; an exponent requires at least one digit.
  [[src/builtins/json_package.mfb:__json_validNumber]]
- **Strings**: raw control characters (code points `< 32`) inside a string are
  rejected. Escapes decode `\" \\ \/ \b \f \n \r \t` and `\uXXXX`. A `\u` high
  surrogate (`U+D800`–`U+DBFF`) must be immediately followed by `\u` and a low
  surrogate (`U+DC00`–`U+DFFF`), combined into one astral code point; a lone or
  mismatched surrogate fails. Hex digits accept both cases.
  [[src/builtins/json_package.mfb:__json_parseUnicodeEscape]]
- The parser is fully recursive (each nested value/array-item/object-member is a
  recursive call), so depth is bounded by the runtime call stack, not an explicit
  limit.

## Stringify output form

`__json_stringify` is a recursive, deterministic serializer producing compact
output — no spaces, no newlines, no indentation.
[[src/builtins/json_package.mfb:__json_stringify]]

| Kind | Output |
| --- | --- |
| `JsonNull` | `null` |
| `JsonBool` | `true` / `false` |
| `JsonNum` | shortest round-trippable form (see below) |
| `JsonStr` | `"` + escaped body + `"` |
| `JsonArr` | `[` items joined by `,` `]` |
| `JsonObj` | `{` `"key":value` members joined by `,` `}` |

Object members are emitted in the iteration order of the underlying `Map` (insertion
order, as the `collections::` map preserves it); keys are escaped the same way as
string values.

### Number formatting

`__json_stringifyNumber` first renders the value with zero fractional digits
(`toString(value, 0)`); if that integer text round-trips back to the same `Float`
(`toFloat(text) = value`), it is emitted as-is (integral values print without a
decimal point). Otherwise the value is rendered with 9 fractional digits and then
trailing zeros — and a trailing `.` — are trimmed by `__json_trimFloatText`. NaN
and ±infinity (`"nan"`, `"-nan"`, `"inf"`, `"-inf"`) are rejected with error
`77050003`, since JSON has no representation for them.
[[src/builtins/json_package.mfb:__json_stringifyNumber]]

### String escaping

`__json_escapeString` iterates graphemes and escapes `"` → `\"`, `\` → `\\`,
`/` → `\/`, newline → `\n`, tab → `\t`, carriage return → `\r`, backspace
(U+0008) → `\b`, form feed (U+000C) → `\f`. Any remaining control character
(code point `< 32`) is emitted as a `\u00XX` escape; all other characters pass
through unchanged (non-ASCII is left as raw UTF-8, not `\u`-escaped). Note the
solidus `/` is always escaped on output even though it is optional in JSON.
[[src/builtins/json_package.mfb:__json_escapeString]]

## Path-based access: `get` / `getOr`

Both accessors take a `Json` root and a `List OF String` *path* of object keys
and walk it left to right. The path addresses object fields only — there is no
array-index step; each path element is looked up as a key in the current value's
`JsonObj.fields`. [[src/builtins/json_package.mfb:__json_get]]

| Step state | `get` | `getOr` |
| --- | --- | --- |
| current is `JsonObj`, key present | descend to field | descend to field |
| current is `JsonObj`, key absent | fail `77050004` | return `defaultValue` |
| current is not `JsonObj` | fail `77050004` ("not found") | return `defaultValue` |
| path exhausted | return current `Json` | return current `Json` |

An empty path returns the root value unchanged. `get` raises error `77050004`
("not found") on any missing key or non-object traversal; `getOr` never fails for
those cases and instead returns the supplied `defaultValue` (itself a `Json`).
The returned value is the full `Json` subtree at the path, including the variant
tag. [[src/builtins/json_package.mfb:__json_getOr]]

## Error codes

| Code | Raised by | Meaning |
| --- | --- | --- |
| `77050003` | parse, stringify-number | invalid JSON format / unrepresentable number |
| `77050004` | `get` | path not found / non-object traversal |

## See Also

* ./mfb man json — per-function API reference
* ./mfb spec architecture frontend — how source packages are injected into the AST
* ./mfb spec architecture monomorphization — instantiation of `List OF Json` / `Map OF String TO Json`
* ./mfb spec memory arenas — `List` and `Map` backing storage
* ./mfb spec language types — the union and record model
* ./mfb spec unicode strings-model — grapheme iteration used by parse and escape
