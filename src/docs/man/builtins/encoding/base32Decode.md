# base32Decode

Decode a standard Base32 `String` into a `List OF Byte`.

## Synopsis

```
encoding::base32Decode(text AS String) AS List OF Byte
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

`encoding::base32Decode` parses `text` as standard Base32 (RFC 4648 §6) and
returns the bytes it encodes. Each character selects a 5-bit value from the
alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`; the values are concatenated
most-significant bit first into a continuous bit stream and emitted eight bits at
a time, so leftover bits that do not fill a final byte are discarded. This is the
inverse of `encoding::base32Encode`. [[src/builtins/encoding_package.mfb:__encoding_base32Decode]] [[src/builtins/encoding_package.mfb:__encoding_baseDecodeBits]]

Decoding is case-insensitive: `A`–`Z` and `a`–`z` map to the same values `0`–`25`,
and the digits `2`–`7` map to `26`–`31`. The `=` character is treated as padding
and may appear only as a trailing run; once a `=` is seen, any later non-padding
character is rejected. Padding characters are otherwise ignored and do not
contribute bits. [[src/builtins/encoding_package.mfb:__encoding_base32Value]]

The total input length (including padding) must be a multiple of eight
characters. In addition, the number of non-padding symbols must correspond to a
valid Base32 group boundary: a symbol count whose remainder modulo eight is `1`,
`3`, or `6` cannot occur in any well-formed Base32 encoding and is rejected. The
empty string decodes to the empty list. [[src/builtins/encoding_package.mfb:__encoding_base32Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The Base32 text to decode. Length must be a multiple of eight; must contain only the alphabet characters `A`–`Z`, `a`–`z`, `2`–`7`, and trailing `=` padding. The empty string is accepted and decodes to the empty list. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The decoded bytes; the empty list for the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `text` has a length that is not a multiple of eight; contains a character outside the Base32 alphabet (`A`–`Z`, `a`–`z`, `2`–`7`, `=`); has a non-padding character following a `=` padding character; or has a non-padding symbol count whose remainder modulo eight is `1`, `3`, or `6`. [[src/builtins/encoding_package.mfb:__encoding_base32Decode]] |

## Examples

Decode a Base32 string back to text:

```
IMPORT encoding

LET bytes AS List OF Byte = encoding::base32Decode("NBSWY3DP")
io::print(encoding::utf8Decode(bytes))
```

Round-trip through `base32Encode`:

```
IMPORT encoding

LET raw AS List OF Byte = encoding::utf8Encode("hello")
LET text AS String = encoding::base32Encode(raw)
io::print(text)
io::print(encoding::utf8Decode(encoding::base32Decode(text)))
```

## See also

- `mfb man encoding base32Encode`
- `mfb man encoding base64Decode`
- `mfb man encoding hexDecode`
- `mfb man encoding utf8Decode`
- `mfb man encoding`
