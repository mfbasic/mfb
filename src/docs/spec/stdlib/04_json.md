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
[[src/codegen/builtins/json/mod.rs:Json]]

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
[[src/codegen/builtins/json/func_stringify.rs:__json_stringify]]

### Numbers are always `Float`

JSON has a single numeric type and the model follows: every number — integral or
fractional — is stored in `JsonNum.value` as a 64-bit `Float`. There is no
`JsonInt`. Parsing converts the lexed numeric token to `Float` via `toFloat`;
out-of-range or unparseable tokens fail (see grammar).
[[src/codegen/builtins/json/helper_parse_number.rs:__json_parseNumber]]

## AST injection (front-end seam)

`json::` is not linked as a precompiled object. Its behaviour is injected MFBASIC
source: when the program imports `json`, the parsed package file is appended into
the project AST before the rest of the front end runs. The augmented project clones the original and
pushes `source_file()` (the parsed `json_package.mfb`) only if `uses_package`
finds an `IMPORT json`; otherwise the project is returned unchanged. The package
source then flows through the same resolver / monomorphization / codegen path as
user code. [[src/codegen/builtins/json/mod.rs:augmented_project]]

The seam also models the four public calls (`json.parse`, `json.stringify`,
`json.get`, `json.getOr`; `parse` and `stringify` each carry overloads) for type
resolution: `resolve_call` maps an exact
argument-type signature to a return type, and `implementation_name` rewrites each
public call to its `__json_*` source FUNC. The `Json*` family is registered as
built-in types, and `is_json_value_type` treats `Json` and all six variant record
names as acceptable wherever a `Json` argument is expected (so a bare `JsonObj`
may be passed where `Json` is wanted). [[src/codegen/builtins/json/mod.rs:resolve_call]]
[[src/codegen/builtins/json/mod.rs:is_json_value_type]]

See `./mfb spec architecture frontend` for the injection ordering and
`./mfb spec architecture monomorphization` for how the generic `List OF Json` /
`Map OF String TO Json` instantiations are produced.

## Parse acceptance grammar

`__json_parse` takes the input's UTF-8 bytes (`strings::toBytes`), skips leading
whitespace, parses one value, skips trailing whitespace, and requires the byte
cursor to be exactly at end-of-input; any trailing non-whitespace fails. Most
failures raise error `77050003` ("invalid JSON format").
[[src/codegen/builtins/json/func_parse.rs:__json_parse]]

The scanners index bytes, not grapheme clusters. Every structural character and
every whitespace character JSON defines is ASCII, so a byte compare is exact and
the scan never splits a multi-byte scalar: a byte `>= 128` occurs only inside a
string, where it is copied through verbatim into the accumulated `List OF Byte`
that becomes the `String` at the closing quote, or inside a number token, where
the grammar check rejects it. This is also what makes a CR LF pair two whitespace
bytes rather than one grapheme cluster that matches neither `\r` nor `\n`.

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
everything else to the number lexer. [[src/codegen/builtins/json/func_parse.rs:__json_parse]]

Notable parse rules and deviations:

- **Numbers**: a number token is collected greedily up to the next `,`, `]`, `}`,
  or whitespace, then validated by `__json_validNumber` against the grammar above
  *before* `toFloat` conversion. The exponent marker accepts both `e` and `E`; a
  leading `0` may not be followed by more integer digits; a fraction requires at
  least one digit after `.`; an exponent requires at least one digit.
  [[src/codegen/builtins/json/helper_valid_number.rs:__json_validNumber]]
- **Strings**: raw control characters (code points `< 32`) inside a string are
  rejected. Escapes decode `\" \\ \/ \b \f \n \r \t` and `\uXXXX`. A `\u` high
  surrogate (`U+D800`–`U+DBFF`) must be immediately followed by `\u` and a low
  surrogate (`U+DC00`–`U+DFFF`), combined into one astral code point; a lone or
  mismatched surrogate fails. Hex digits accept both cases.
  [[src/codegen/builtins/json/helper_parse_unicode_escape.rs:__json_parseUnicodeEscape]]
- Sibling array items and object members are accumulated iteratively, but each
  level of structural *nesting* is a recursive call, so nesting depth would
  otherwise be bounded only by the runtime call stack. Because MFBASIC has no
  tail-call optimization, an adversarially deep document would overflow that stack
  and crash the process, so the parser caps structural nesting at an explicit fixed
  depth (256 levels of arrays and objects combined) and rejects anything deeper
  with `77050024` (`ErrDepthExceeded`), which is deliberately distinct from
  `77050003`: the text is well-formed JSON, it is simply nested deeper than this
  reader descends, and a caller can act on that.
  [[src/codegen/builtins/json/helper_parse_value.rs:__json_parseValue]]

## Stringify output form

`__json_stringify` is a recursive, deterministic serializer producing compact
output — no spaces, no newlines, no indentation.
[[src/codegen/builtins/json/func_stringify.rs:__json_stringify]]

`json::stringify` also has two indented overloads, `stringify(value, count)` and
`stringify(value, indent)`, which render through `__json_stringifyIndent` — a
depth-carrying twin of the walk below that emits `\n` and one indent per level,
with `": "` after each object key. Empty arrays and objects stay inline (`[]`,
`{}`) at every depth. A count clamps to `0..=10` and a string indent is truncated
to its first 10 characters; `0` and `""` mean compact and are byte-identical to
the one-argument form. The leaf renderings (numbers, escaped strings) are shared
with the compact body rather than reimplemented, so the rules below hold in both.
[[src/codegen/builtins/json/helper_stringify_indent.rs:__json_stringifyIndent]]

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

Numbers are written exactly as ECMAScript's `Number::toString` writes them, so a
document produced here is byte-comparable with one produced by `JSON.stringify`.

The digits come from significant-digit machinery, not from a count of decimal
places. `json::sciParts` returns the first 18 significant digits of the
magnitude — truncated, never rounded — together with the decimal exponent and a
sticky flag saying whether anything non-zero follows, encoded as
`"<sticky><18 digits>e<exponent>"`. That single call then serves the whole
search: `__json_stringifyNumber` rounds those digits to `p` places for
`p = 1..17`, and takes the first rendering that reads back as the same `Float`.
Rounding an 18-digit truncation at `p`, with the sticky recomputed from the
digits being dropped, is exactly rounding the true value at `p`, so nothing is
lost by performing it on text.

Rounding is **half-to-even**. Where two equally short renderings both read back
exactly, ECMA-262 requires the candidate whose last digit is even, and rounding
half away from zero instead would disagree with JavaScript on a small fraction
of values.

Placement follows ECMAScript. Writing `n` for `exponent + 1`: `1 <= n <= 21`
prints plainly, with the point inside the digits or zeros padded out to it;
`-6 < n <= 0` prints `0.` then `-n` zeros then the digits; anything else prints
exponentially with one digit before the point and an explicit, unpadded exponent
sign. So `1e20` is `100000000000000000000` and `1e21` is `1e+21`; `0.000001` is
plain and `1e-7` is not. A whole number needs no special case: it falls out of
the `n >= count` branch, which pads the digits out to the point.

Negative zero is mapped to `0`, matching `JSON.stringify(-0)`; the native
formatter itself keeps the sign, and that is untouched.

**Every finite `Float` has a rendering.** 17 significant digits identify a
binary64, so the search always succeeds; the `FAIL` at the end of the body is an
invariant guard that cannot be reached. This is a change: rendering used to
search fixed-point forms of up to 25 decimal places, so a value too small to
reach a significant digit within 25 places — `1e-30`, `5e-324` — had no
representation at all and the call failed.

NaN and ±infinity are rejected before any of this, with their own codes —
`77050013` (`ErrFloatNaN`) and `77050014` (`ErrFloatInf`) — since JSON has no
representation for them.
[[src/codegen/builtins/json/helper_stringify_number.rs:__json_stringifyNumber]]
[[src/codegen/builtins/json/helper_round_digits.rs:__json_roundDigits]]
[[src/codegen/builtins/json/helper_place_digits.rs:__json_placeDigits]]
[[src/codegen/string/format/float_format_sci.rs:lower_float_to_string_sci_helpers]]
[[src/codegen/builtins/json/helper_require_finite_number_text.rs:__json_requireFiniteNumberText]]

### String escaping

`__json_escapeString` iterates graphemes and escapes `"` → `\"`, `\` → `\\`,
newline → `\n`, tab → `\t`, carriage return → `\r`, backspace (U+0008) → `\b`,
form feed (U+000C) → `\f`. Any remaining control character (code point `< 32`)
is emitted as a `\u00XX` escape; all other characters pass through unchanged
(non-ASCII is left as raw UTF-8, not `\u`-escaped).

The solidus `/` is **not** escaped on output. Escaping it is permitted by JSON
but not required, and `JSON.stringify` does not do it, so a document produced
here is byte-comparable with one produced by JavaScript. `\/` is still *accepted*
on input, since it remains valid JSON.
[[src/codegen/builtins/json/helper_escape_string.rs:__json_escapeString]]

## Revival: `parse(text, reviver)`

The two-argument `parse` runs `__json_parse` to completion and then walks the
finished tree through `__json_revive`, calling the caller's
`FUNC(String, Json) AS Json` once per value and storing what it returns in place.
The walk is post-order, so a container is revived after its elements or members
and receives the already-revived children; the document root is revived last,
under the key `""`. An array element's key is its index rendered as a decimal
string, an object member's key is its name.

Because revival runs after parsing rather than during it, a malformed document
fails before the reviver is called at all, and every parse helper is untouched.
Duplicate keys have already collapsed last-wins into the map, so the reviver sees
each surviving key once. There is no deletion: MFBASIC has no `undefined`, so
returning `JsonNull[NOTHING]` stores a JSON null rather than dropping the member —
the one divergence from `JSON.parse`'s reviver.
[[src/codegen/builtins/json/helper_revive.rs:__json_revive]]

## Path-based access: `get` / `getOr`

Both accessors take a `Json` root and a `List OF String` *path* and walk it left
to right. What a path element means depends only on the variant underfoot: on a
`JsonObj` it is an object key matched exactly, and on a `JsonArr` it is a
zero-based decimal index, the way RFC 6901 spells one.

The index grammar is strict — an optional `0` alone, or a nonzero digit followed
by digits, at most 18 characters. A leading `+` or `-`, a leading zero such as
`01`, whitespace, and anything non-numeric are all rejected, and the digits are
matched by code point rather than by string equality so a decorated grapheme such
as `1` followed by a combining mark is not mistaken for a digit. A token that is
not a valid index simply misses, which is what keeps `getOr`'s never-fails
contract intact.

A key that looks like a number is still a key on an object, so no program that
worked before changes behaviour: reaching an array used to fail unconditionally.
[[src/codegen/builtins/json/func_get.rs:__json_get]]
[[src/codegen/builtins/json/helper_array_index.rs:__json_arrayIndex]]

| Step state | `get` | `getOr` |
| --- | --- | --- |
| current is `JsonObj`, key present | descend to field | descend to field |
| current is `JsonObj`, key absent | fail `77050004` | return `defaultValue` |
| current is `JsonArr`, index in range | descend to element | descend to element |
| current is `JsonArr`, index out of range | fail `77050004` | return `defaultValue` |
| current is `JsonArr`, token is not a valid index | fail `77050004` | return `defaultValue` |
| current is a scalar variant | fail `77050004` ("not found") | return `defaultValue` |
| path exhausted | return current `Json` | return current `Json` |

An empty path returns the root value unchanged. `get` raises error `77050004`
("not found") on any missing key, out-of-range or malformed index, or traversal
into a scalar; `getOr` never fails for those cases and instead returns the
supplied `defaultValue` (itself a `Json`).
The returned value is the full `Json` subtree at the path, including the variant
tag. [[src/codegen/builtins/json/func_get_or.rs:__json_getOr]]

## Error codes

| Code | Constant | Raised by | Meaning |
| --- | --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | parse | invalid JSON format. `stringify` retains the code as an unreachable invariant guard: since plan-120-G every finite `Float` renders |
| `77050004` | `ErrNotFound` | `get` | path not found: missing key, bad or out-of-range index, or traversal into a scalar |
| `77050010` | `ErrOverflow` | parse | a valid JSON number with no `Float` anywhere near it (`1e400`); re-raised from `toFloat` rather than swallowed |
| `77050013` | `ErrFloatNaN` | stringify | NaN has no JSON representation |
| `77050014` | `ErrFloatInf` | stringify | ±infinity has no JSON representation |
| `77050024` | `ErrDepthExceeded` | parse | well-formed but nested deeper than 256 levels |
| `77050025` | `ErrInvalidSurrogate` | parse | a `\u` escape naming an unpaired surrogate |

An error raised by a `parse` reviver is not caught and surfaces at the call site
with whatever code the reviver used, so this table is not exhaustive for the
two-argument form.

## See Also

* ./mfb man json — per-function API reference
* ./mfb spec architecture frontend — how source packages are injected into the AST
* ./mfb spec architecture monomorphization — instantiation of `List OF Json` / `Map OF String TO Json`
* ./mfb spec memory arenas — `List` and `Map` backing storage
* ./mfb spec language types — the union and record model
* ./mfb spec unicode strings-model — the byte/scalar/grapheme layers; parse scans UTF-8 bytes, escape iterates graphemes
