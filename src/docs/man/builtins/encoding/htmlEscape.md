# htmlEscape

Escape the five HTML/XML metacharacters in a `String`.

## Synopsis

```
encoding::htmlEscape(text AS String) AS String
```

## Package

encoding

## Imports

```
IMPORT encoding
```

`encoding` is a built-in package written in MFBASIC source, so no manifest
dependency is required. [[src/builtins/encoding.rs:augmented_project]]

## Description

`encoding::htmlEscape` produces a form of `text` that is safe to embed inside
HTML/XML element content and attribute values. It replaces each of the five
metacharacters with its named character reference:
[[src/builtins/encoding_package.mfb:__encoding_htmlEscape]]

- `&` (ampersand) becomes `&amp;`
- `<` (less-than) becomes `&lt;`
- `>` (greater-than) becomes `&gt;`
- `"` (double quote) becomes `&quot;`
- `'` (apostrophe) becomes `&apos;`

The ampersand is substituted **first**, before the other four, so that the `&`
introduced by each replacement entity is not escaped a second time; the result
is therefore a single, correct level of escaping.
[[src/builtins/encoding_package.mfb:__encoding_htmlEscape]]

Every other character — including whitespace, digits, letters, and non-ASCII
code points — passes through unchanged; only the five characters above are
rewritten. The function is **total**: every `String`, including the empty
string (which yields the empty string), escapes successfully, and it never
raises a runtime error.

The inverse operation is `encoding::htmlUnescape`, which parses named and
numeric character references back into text.
[[src/builtins/encoding_package.mfb:__encoding_htmlUnescape]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The text to escape. Any string, including the empty string, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A copy of `text` with `&`, `<`, `>`, `"`, and `'` replaced by `&amp;`, `&lt;`, `&gt;`, `&quot;`, and `&apos;` respectively; all other characters unchanged. The empty string for empty input. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Escape a fragment before placing it in element content:

```
IMPORT encoding
IMPORT io

io::print(encoding::htmlEscape("<a href='#'>Tom & Jerry</a>"))
```

Round-trip through `htmlUnescape`:

```
IMPORT encoding
IMPORT io

LET esc AS String = encoding::htmlEscape("5 > 3 & 2 < 4")
io::print(esc)
io::print(encoding::htmlUnescape(esc))
```

## See also

- `mfb man encoding htmlUnescape`
- `mfb man encoding percentEncode`
- `mfb man encoding`
