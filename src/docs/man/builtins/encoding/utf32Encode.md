# utf32Encode

Encode a `String` to its UTF-32 code points.

## Synopsis

```
encoding::utf32Encode(value AS String) AS List OF Integer
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

`encoding::utf32Encode` returns the UTF-32 encoding of `value` as a list of
numeric code points, one element per Unicode scalar value. Each scalar is a
number in the range `0..1114111` (`0x10FFFF`); because a valid `String` holds no
surrogate scalars, the result never contains a value in the surrogate range
`55296..57343`. [[src/builtins/encoding_package.mfb:__encoding_utf32Encode]]

The scalars are produced by decoding the string's UTF-8 bytes in order: each
1-to-4-byte sequence contributes exactly one code point, so the returned list
has one element per Unicode scalar in `value` (which may be fewer than its byte
length). [[src/builtins/encoding_package.mfb:__encoding_codepoints]]

These are UTF-32 *code points*, not a byte serialization: the result is a
sequence of numbers, so no byte order (endianness) or byte-order mark applies.
The function is **total** — every string, including the empty string (which
yields an empty list), encodes successfully, and it never raises a runtime
error. The inverse operation is `encoding::utf32Decode`, which turns a
`List OF Integer` of code points back into a `String` and rejects out-of-range
or surrogate code points. [[src/builtins/encoding_package.mfb:__encoding_utf32Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to encode. Any string, including the empty string, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Integer` | The Unicode scalar values of `value`, each in `0..1114111` and never a surrogate; empty for the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a string to its UTF-32 code points:

```
IMPORT encoding

LET points AS List OF Integer = encoding::utf32Encode("hello")
io::print(toString(len(points)))
```

Round-trip an astral scalar (an emoji) through UTF-32:

```
IMPORT encoding

LET points AS List OF Integer = encoding::utf32Encode("😀")
io::print(encoding::utf32Decode(points))
```

## See also

- `mfb man encoding utf32Decode`
- `mfb man encoding utf16Encode`
- `mfb man encoding utf8Encode`
- `mfb man encoding`
- `mfb man unicode`
