# utf32Decode

Decode a `List OF Integer` of UTF-32 code points to a `String`.

## Synopsis

```
encoding::utf32Decode(value AS List OF Integer) AS String
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

`encoding::utf32Decode` interprets `value` as a sequence of UTF-32 code points
and returns the corresponding text. Each element is a full Unicode scalar value:
because UTF-32 is a fixed-width encoding, one list element decodes directly to
one scalar, with no multi-unit sequences or surrogate pairs to combine. The empty
list decodes to the empty string. [[src/builtins/encoding_package.mfb:__encoding_utf32Decode]]

Every element must be a valid Unicode scalar. A code point is rejected when it is
negative or greater than `1114111` (`0x10FFFF`), or when it lies in the surrogate
range `55296..57343` (`0xD800..0xDFFF`) — surrogates are not scalar values and
cannot appear on their own in UTF-32. Any such element fails rather than
producing replacement text. The elements are treated as numeric code points, not
a byte serialization, so no byte order (endianness) or byte-order mark applies.
[[src/builtins/encoding_package.mfb:__encoding_utf32Decode]]

`utf32Decode` is the inverse of `encoding::utf32Encode`: decoding the code points
that `utf32Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions.
[[src/builtins/encoding_package.mfb:__encoding_utf32Encode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF Integer` | The UTF-32 code points to decode. Every element must be in `0..1114111` and must not be a surrogate (`55296..57343`). [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text; the empty string for an empty input list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | An element is negative, greater than `1114111`, or a surrogate code point in `55296..57343`. [[src/builtins/encoding_package.mfb:__encoding_utf32Decode]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Decode UTF-32 code points back to text:

```
IMPORT encoding

io::print(encoding::utf32Decode([104, 105]))
```

Round-trip an astral scalar (an emoji) through UTF-32:

```
IMPORT encoding

LET points AS List OF Integer = encoding::utf32Encode("😀")
io::print(encoding::utf32Decode(points))
```

## See also

- `mfb man encoding utf32Encode`
- `mfb man encoding utf16Decode`
- `mfb man encoding utf8Decode`
- `mfb man encoding`
- `mfb man unicode`
