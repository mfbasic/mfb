# base32Encode

Encode a `List OF Byte` to a standard Base32 `String`.

## Synopsis

```
encoding::base32Encode(data AS List OF Byte) AS String
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

`encoding::base32Encode` returns the standard Base32 representation of `data`
as defined by RFC 4648 §6. Input bytes are consumed as a continuous bit stream,
most-significant bit first, and emitted five bits at a time; each 5-bit group
selects one character from the uppercase alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`. [[src/builtins/encoding_package.mfb:__encoding_base32Encode]]

Encoding operates on 40-bit (5-byte) groups, each producing eight Base32
characters. When the final group is short, its remaining bits become the high
bits of a last symbol and are zero-filled at the low end, then the output is
padded with `=` characters until its length is a multiple of eight, so the
result length is always a multiple of eight. An empty list yields the empty
string. [[src/builtins/encoding_package.mfb:__encoding_baseEncode]]

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse operation
is `encoding::base32Decode`, which parses a Base32 string back into a
`List OF Byte`. [[src/builtins/encoding_package.mfb:__encoding_base32Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `data` | `List OF Byte` | The bytes to encode. Any list of bytes, including the empty list, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The uppercase Base32 encoding of `data` with `=` padding to a multiple of eight characters; the empty string for an empty list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode bytes to Base32:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base32Encode(raw))
END SUB
```

Round-trip through `base32Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base32Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base32Decode(text)))
END SUB
```

## See also

- `mfb man encoding base32Decode`
- `mfb man encoding base64Encode`
- `mfb man encoding hexEncode`
- `mfb man encoding utf8Encode`
- `mfb man encoding`
