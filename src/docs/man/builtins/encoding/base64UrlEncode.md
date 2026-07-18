# base64UrlEncode

Encode a `List OF Byte` to a URL- and filename-safe Base64 `String`.

## Synopsis

```
encoding::base64UrlEncode(data AS List OF Byte) AS String
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

`encoding::base64UrlEncode` returns the URL- and filename-safe Base64
representation of `data` as defined by RFC 4648 §5. Input bytes are consumed as
a continuous bit stream, most-significant bit first, and emitted six bits at a
time; each 6-bit group selects one character from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_`, so the
result uses `-` and `_` for the final two symbols instead of the `+` and `/`
used by the standard variant. [[src/builtins/encoding_package.mfb:__encoding_base64UrlEncode]]

Encoding operates on 24-bit (3-byte) groups, each producing four Base64
characters. When the final group is short, the remaining data bits occupy the
high-order bits of the last symbol and the low-order bits are zero-filled, but
**no** `=` padding characters are appended, so the output length is not rounded
up to a multiple of four. This is the difference from `encoding::base64Encode`,
which pads with `=`. An empty list yields the empty
string. [[src/builtins/encoding_package.mfb:__encoding_baseEncode]]

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse
operation is `encoding::base64UrlDecode`, which parses a URL-safe Base64 string
back into a `List OF Byte`. [[src/builtins/encoding_package.mfb:__encoding_base64UrlDecode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `data` | `List OF Byte` | The bytes to encode. Any list of bytes, including the empty list, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The URL-safe Base64 encoding of `data` with `-`/`_` symbols and no `=` padding; the empty string for an empty list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode bytes to URL-safe Base64:

```
IMPORT encoding

LET raw AS List OF Byte = encoding::utf8Encode("hi")
io::print(encoding::base64UrlEncode(raw))
```

Round-trip through `base64UrlDecode`:

```
IMPORT encoding

LET raw AS List OF Byte = encoding::utf8Encode("hello")
LET text AS String = encoding::base64UrlEncode(raw)
io::print(text)
io::print(encoding::utf8Decode(encoding::base64UrlDecode(text)))
```

## See also

- `mfb man encoding base64UrlDecode`
- `mfb man encoding base64Encode`
- `mfb man encoding base32Encode`
- `mfb man encoding hexEncode`
- `mfb man encoding utf8Encode`
- `mfb man encoding`
