# hexEncode

Encode a `List OF Byte` to a lowercase hexadecimal `String`.

## Synopsis

```
encoding::hexEncode(data AS List OF Byte) AS String
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

`encoding::hexEncode` returns the base-16 representation of `data`, emitting two
lowercase hexadecimal characters for every input byte with no separators, prefix,
or padding. Bytes are encoded in order: byte value `v` becomes the digit for
`v / 16` followed by the digit for the low nibble, drawn from the alphabet
`0123456789abcdef`. [[src/builtins/encoding_package.mfb:__encoding_hexEncode]]

The result length is always exactly twice the number of input bytes. An empty
list yields the empty string. Use `strings::upper` on the result if uppercase hex
is required. [[src/builtins/encoding_package.mfb:__encoding_hexDigit]]

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse operation
is `encoding::hexDecode`, which parses a hex string (accepting upper- or
lowercase digits) back into a `List OF Byte`. [[src/builtins/encoding_package.mfb:__encoding_hexDecode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `data` | `List OF Byte` | The bytes to encode. Any list of bytes, including the empty list, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The lowercase hex encoding of `data`, two characters per byte with no separators; the empty string for an empty list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors. Every `List OF Byte` encodes successfully: each byte's nibbles are always in `0`–`15`, valid indices into the 16-character alphabet, so no failure path exists. [[src/builtins/encoding_package.mfb:__encoding_hexEncode]]

## Examples

Encode bytes to lowercase hex:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::hexEncode(raw))
END SUB
```

Round-trip through `hexDecode`, and uppercase the digits:

```
IMPORT encoding
IMPORT strings
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  LET hex AS String = encoding::hexEncode(raw)
  io::print(strings::upper(hex))
  io::print(encoding::utf8Decode(encoding::hexDecode(hex)))
END SUB
```

## See also

- `mfb man encoding hexDecode`
- `mfb man encoding base64Encode`
- `mfb man encoding utf8Encode`
- `mfb man strings upper`
- `mfb man encoding`
