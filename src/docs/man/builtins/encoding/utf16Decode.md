# utf16Decode

Decode a `List OF Integer` of UTF-16 code units to a `String`.

## Synopsis

```
encoding::utf16Decode(value AS List OF Integer) AS String
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

`encoding::utf16Decode` interprets `value` as a sequence of UTF-16 code units and
returns the corresponding text. Each element is examined in order: a unit in the
Basic Multilingual Plane decodes to a single Unicode scalar, while a high
surrogate in `55296..56319` is combined with the following low surrogate in
`56320..57343` to reconstruct one astral scalar. The empty list decodes to the
empty string. [[src/builtins/encoding_package.mfb:__encoding_utf16Decode]]

A surrogate pair is recombined by subtracting the surrogate offsets, shifting the
high unit up by ten bits, adding the low ten bits, and adding `65536`, yielding a
scalar above `65535`. [[src/builtins/encoding_package.mfb:__encoding_utf16Decode]]

Every element must lie in `0..65535`; a value outside that range is rejected. A
high surrogate that is the last element, or is followed by a unit that is not a
low surrogate, is an unpaired surrogate, as is a low surrogate that does not
follow a high surrogate — all of these fail rather than producing replacement
text. The units are treated as numeric code units, not a byte serialization, so
no byte order (endianness) or byte-order mark applies.
[[src/builtins/encoding_package.mfb:__encoding_utf16Decode]]

`utf16Decode` is the inverse of `encoding::utf16Encode`: decoding the code units
that `utf16Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions.
[[src/builtins/encoding.rs:UTF16_DECODE]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF Integer` | The UTF-16 code units to decode. Every element must be in `0..65535`, and surrogates must be correctly paired. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text; the empty string for an empty input list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | An element is outside `0..65535`, or the code units contain an unpaired surrogate. [[src/builtins/encoding_package.mfb:__encoding_utf16Decode]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Decode UTF-16 code units back to text:

```
IMPORT encoding

io::print(encoding::utf16Decode([104, 105]))
```

Round-trip an astral scalar (an emoji) through UTF-16:

```
IMPORT encoding

LET units AS List OF Integer = encoding::utf16Encode("😀")
io::print(encoding::utf16Decode(units))
```

## See also

- `mfb man encoding utf16Encode`
- `mfb man encoding utf8Decode`
- `mfb man encoding utf32Decode`
- `mfb man encoding`
- `mfb man unicode`
