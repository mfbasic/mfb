# utf16Encode

Encode a `String` to its UTF-16 code units.

## Synopsis

```
encoding::utf16Encode(value AS String) AS List OF Integer
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

`encoding::utf16Encode` returns the UTF-16 encoding of `value` as a list of
numeric code units, one element per 16-bit unit. Each Unicode scalar in `value`
is examined in order: a scalar in the Basic Multilingual Plane (`0..65535`)
becomes a single code unit, and an astral scalar (above `65535`) is split into a
surrogate pair — a high surrogate in `55296..56319` followed by a low surrogate
in `56320..57343`. [[src/builtins/encoding_package.mfb:__encoding_utf16Encode]]

The surrogate split subtracts `65536` from the scalar, then takes the top ten
bits (offset by `55296`) as the high unit and the low ten bits (offset by
`56320`) as the low unit, so every returned element lies in `0..65535`.
[[src/builtins/encoding_package.mfb:__encoding_utf16Encode]]

These are UTF-16 *code units*, not a byte serialization: the result is a
sequence of numbers, so no byte order (endianness) or byte-order mark applies.
The function is **total** — every string, including the empty string (which
yields an empty list), encodes successfully, and it never raises a runtime
error. The inverse operation is `encoding::utf16Decode`, which turns a
`List OF Integer` of code units back into a `String` and rejects unpaired
surrogates and out-of-range units. [[src/builtins/encoding.rs:UTF16_ENCODE]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to encode. Any string, including the empty string, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Integer` | The UTF-16 code units of `value`, each in `0..65535`; empty for the empty string. Astral scalars contribute two elements (a surrogate pair). [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a string to its UTF-16 code units:

```
IMPORT encoding

LET units AS List OF Integer = encoding::utf16Encode("hello")
io::print(toString(collections::len(units)))
```

Round-trip an astral scalar (an emoji) through UTF-16:

```
IMPORT encoding

LET units AS List OF Integer = encoding::utf16Encode("😀")
io::print(encoding::utf16Decode(units))
```

## See also

- `mfb man encoding utf16Decode`
- `mfb man encoding utf8Encode`
- `mfb man encoding utf32Encode`
- `mfb man encoding`
- `mfb man unicode`
