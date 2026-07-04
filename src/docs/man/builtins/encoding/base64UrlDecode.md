# base64UrlDecode

Decode a URL- and filename-safe Base64 `String` into a `List OF Byte`.

## Synopsis

```
encoding::base64UrlDecode(text AS String) AS List OF Byte
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

`encoding::base64UrlDecode` parses `text` as URL- and filename-safe Base64
(RFC 4648 §5) and returns the bytes it encodes. Each character selects a 6-bit
value from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_`; the values
are concatenated most-significant bit first into a continuous bit stream and
emitted eight bits at a time, so leftover bits that do not fill a final byte are
discarded. This is the inverse of `encoding::base64UrlEncode`. [[src/builtins/encoding_package.mfb:__encoding_base64UrlDecode]] [[src/builtins/encoding_package.mfb:__encoding_baseDecodeBits]]

The alphabet is the URL-safe variant using `-` and `_` for values `62` and `63`;
it is case-sensitive (`A`–`Z` map to `0`–`25`, `a`–`z` to `26`–`51`, `0`–`9` to
`52`–`61`). The `=` character is treated as padding: once a `=` is seen, any
later non-padding character is rejected. Padding characters are otherwise ignored
and contribute no bits. [[src/builtins/encoding_package.mfb:__encoding_base64Value]] [[src/builtins/encoding_package.mfb:__encoding_base64Symbols]]

Unlike `encoding::base64Decode`, this function does **not** require the total
input length to be a multiple of four, so URL-safe text produced without `=`
padding decodes directly; text that does carry `=` padding is also accepted. The
only length constraint is that the number of non-padding symbols cannot be
exactly one more than a multiple of four (a symbol count whose remainder modulo
four is `1`), because no well-formed Base64 group ends on a single 6-bit symbol.
The empty string decodes to the empty list. For the standard variant that uses
`+` and `/`, use `encoding::base64Decode`. [[src/builtins/encoding_package.mfb:__encoding_base64UrlDecode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The URL-safe Base64 text to decode. Must contain only the alphabet characters `A`–`Z`, `a`–`z`, `0`–`9`, `-`, `_`, and optional trailing `=` padding. The empty string is accepted and decodes to the empty list. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The decoded bytes; the empty list for the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `text` contains a character outside the URL-safe Base64 alphabet (`A`–`Z`, `a`–`z`, `0`–`9`, `-`, `_`, `=`); has a non-padding character following a `=` padding character; or has a non-padding symbol count whose remainder modulo four is `1`. [[src/builtins/encoding_package.mfb:__encoding_base64UrlDecode]] [[src/builtins/encoding_package.mfb:__encoding_base64Symbols]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Decode a URL-safe Base64 string (no padding) back to text:

```
IMPORT encoding

LET bytes AS List OF Byte = encoding::base64UrlDecode("aGVsbG8")
io::print(encoding::utf8Decode(bytes))
```

Round-trip through `base64UrlEncode`:

```
IMPORT encoding

LET raw AS List OF Byte = encoding::utf8Encode("hello")
LET text AS String = encoding::base64UrlEncode(raw)
io::print(text)
io::print(encoding::utf8Decode(encoding::base64UrlDecode(text)))
```

## See also

- `mfb man encoding base64UrlEncode`
- `mfb man encoding base64Decode`
- `mfb man encoding base32Decode`
- `mfb man encoding hexDecode`
- `mfb man encoding utf8Decode`
- `mfb man encoding`
