# base64Encode

Encode a `List OF Byte` to a standard Base64 `String`.

## Synopsis

```
encoding::base64Encode(data AS List OF Byte) AS String
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

`encoding::base64Encode` returns the standard Base64 representation of `data`
as defined by RFC 4648 §4. Input bytes are consumed as a continuous bit stream,
most-significant bit first, and emitted six bits at a time; each 6-bit group
selects one character from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`, so the
result uses `+` and `/` for the final two symbols. [[src/builtins/encoding_package.mfb:__encoding_base64Encode]]

Encoding operates on 24-bit (3-byte) groups, each producing four Base64
characters. When the final group is short, the remaining data bits occupy the
high-order bits of the last symbol and the low-order bits are zero-filled, and
the output is then padded with `=` characters until its length is a multiple of
four, so the result length is always a multiple of four. An empty list yields
the empty string. [[src/builtins/encoding_package.mfb:__encoding_baseEncode]]

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. For the URL- and
filename-safe variant that uses `-` and `_` without `=` padding, use
`encoding::base64UrlEncode`. The inverse operation is `encoding::base64Decode`,
which parses a Base64 string back into a `List OF Byte`. [[src/builtins/encoding_package.mfb:__encoding_base64Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `data` | `List OF Byte` | The bytes to encode. Any list of bytes, including the empty list, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The standard Base64 encoding of `data` with `+`/`/` symbols and `=` padding to a multiple of four characters; the empty string for an empty list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode bytes to Base64:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base64Encode(raw))
END SUB
```

Round-trip through `base64Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64Decode(text)))
END SUB
```

## See also

- `mfb man encoding base64Decode`
- `mfb man encoding base64UrlEncode`
- `mfb man encoding base32Encode`
- `mfb man encoding hexEncode`
- `mfb man encoding utf8Encode`
- `mfb man encoding`
