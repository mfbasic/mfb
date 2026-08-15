# hexDecode

Decode a hexadecimal `String` into a `List OF Byte`.

## Synopsis

```
encoding::hexDecode(text AS String) AS List OF Byte
```

## Package

encoding

## Imports

```
IMPORT encoding
```

`encoding` is a built-in package written in MFBASIC source, so no manifest
dependency is required. [[src/codegen/builtins/encoding/mod.rs:augmented_project]]

## Description

`encoding::hexDecode` parses `text` as base-16 and returns the bytes it encodes.
Every two hexadecimal characters produce one byte: the first character is the
high nibble and the second is the low nibble, so the byte value is
`high * 16 + low`. Characters are consumed in order with no separators, prefix,
or padding recognized. [[src/codegen/builtins/encoding/func_hex_decode.rs:__encoding_hexDecode]]

Both cases are accepted for the letter digits: `0`–`9`, `a`–`f`, and `A`–`F` are
valid, and lowercase and uppercase may be mixed freely within the same string.
Any other character is rejected. [[src/codegen/builtins/encoding/package.mfb:__encoding_hexValue]]

The input length must be even, because each byte needs a pair of digits. The
empty string decodes to the empty list. The result always contains exactly half
as many bytes as there are input characters. This is the inverse of
`encoding::hexEncode`, which emits lowercase hex; decoding then re-encoding a
valid string reproduces its lowercase form. [[src/codegen/builtins/encoding/func_hex_decode.rs:register]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The hexadecimal text to decode. Must have even length and contain only the digits `0`–`9`, `a`–`f`, `A`–`F`. The empty string is accepted and decodes to the empty list. [[src/codegen/builtins/encoding/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The decoded bytes, one per pair of input digits; the empty list for the empty string. [[src/codegen/builtins/encoding/func_hex_decode.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `text` has an odd number of characters, or contains a character that is not a hexadecimal digit (`0`–`9`, `a`–`f`, `A`–`F`). [[src/codegen/builtins/encoding/func_hex_decode.rs:__encoding_hexDecode]] |

## Examples

Decode a hex string to bytes and back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::hexDecode("68656c6c6f")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `hexEncode`, mixing digit case on input:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  LET hex AS String = encoding::hexEncode(raw)
  io::print(hex)
  io::print(encoding::utf8Decode(encoding::hexDecode("6869")))
END SUB
```

## See also

- `mfb man encoding hexEncode`
- `mfb man encoding base64Decode`
- `mfb man encoding utf8Decode`
- `mfb man encoding`
