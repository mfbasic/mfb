# htmlUnescape

Decode HTML/XML named and numeric character references in a `String` back to text.

## Synopsis

```
encoding::htmlUnescape(text AS String) AS String
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

`encoding::htmlUnescape` scans `text` grapheme by grapheme and replaces each
character reference — a run that begins with `&` and ends at the next `;` — with
the character it denotes. Every other character, including `&` characters that
are part of a valid reference's expansion, passes through unchanged.
[[src/builtins/encoding_package.mfb:__encoding_htmlUnescape]]

Three reference forms are recognized, distinguished by the text between `&`
and `;`: [[src/builtins/encoding_package.mfb:__encoding_htmlUnescape]]

- A **hexadecimal numeric** reference `&#x…;` or `&#X…;` (for example
  `&#xE9;`), where the digits after `#x`/`#X` are parsed as base 16.
  [[src/builtins/encoding_package.mfb:__encoding_parseHex]]
- A **decimal numeric** reference `&#…;` (for example `&#233;`), where the
  digits after `#` are parsed as base 10.
  [[src/builtins/encoding_package.mfb:__encoding_parseDecimal]]
- A **named** reference `&…;` (for example `&eacute;`), looked up in the
  built-in entity table. [[src/builtins/encoding_package.mfb:__encoding_htmlEntity]]

The resolved code point is emitted as UTF-8 text. Any code point in the range
`0`–`1114111` (`0x10FFFF`) is accepted, including surrogate values, which are
not screened out. [[src/builtins/encoding_package.mfb:__encoding_fromCodepoint]]

The function is **not total**: it fails on a reference that has no `;`
terminator, on a numeric reference whose digits are empty or non-numeric, on an
unknown entity name, and on a numeric reference whose value exceeds `1114111`.
The empty string yields the empty string. `encoding::htmlUnescape` is the
inverse of `encoding::htmlEscape`.
[[src/builtins/encoding_package.mfb:__encoding_htmlUnescape]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The text to decode. Any string, including the empty string, is accepted; well-formed references are expanded and all other characters pass through. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A copy of `text` with each named and numeric character reference replaced by its character. The empty string for empty input. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `&` reference is not closed by a `;`; a numeric reference has empty or non-digit digits; a named reference is unknown; or a numeric reference resolves to a value above `1114111`. [[src/builtins/encoding_package.mfb:__encoding_htmlUnescape]] [[src/builtins/encoding_package.mfb:__encoding_fromCodepoint]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Decode named references:

```
IMPORT encoding
IMPORT io

io::print(encoding::htmlUnescape("&lt;a&gt;"))
```

Decode decimal and hexadecimal numeric references:

```
IMPORT encoding
IMPORT io

io::print(encoding::htmlUnescape("caf&#233; / caf&#xE9;"))
```

Round-trip through `htmlEscape`:

```
IMPORT encoding
IMPORT io

LET esc AS String = encoding::htmlEscape("5 > 3 & 2 < 4")
io::print(encoding::htmlUnescape(esc))
```

## See also

- `mfb man encoding htmlEscape`
- `mfb man encoding percentDecode`
- `mfb man encoding`
