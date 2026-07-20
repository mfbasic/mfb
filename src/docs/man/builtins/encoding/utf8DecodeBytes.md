# utf8DecodeBytes

Decode a `List OF Byte` of UTF-8 octets to a `String`.

## Synopsis

```
encoding::utf8DecodeBytes(value AS List OF Byte) AS String
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

`encoding::utf8DecodeBytes` interprets `value` as a UTF-8 byte sequence and
returns the corresponding text. Because MFBASIC strings are always well-formed
UTF-8, the input is validated in full before the string is produced: the bytes
must form a well-formed UTF-8 sequence, with no invalid, overlong, or truncated
byte sequence. If validation succeeds, the octets are returned verbatim as the
string's storage. The empty list decodes to the empty string.
[[src/builtins/encoding_package.mfb:__encoding_utf8DecodeBytes]]

This is the byte-typed form of `encoding::utf8Decode`. `utf8Decode` is a
parameter overload that selects between a `List OF Byte` and a `List OF Integer`
argument at compile time; `utf8DecodeBytes` is the concrete, non-overloaded name
that always takes a `List OF Byte`, so no overload resolution is involved. The
integer-typed counterpart is `encoding::utf8DecodeInts`, which additionally
requires every element to be in `0..255` before decoding.
[[src/builtins/encoding.rs:UTF8_DECODE_BYTES]] [[src/builtins/encoding.rs:resolve_overload_target]]

It is the inverse of `encoding::utf8EncodeBytes`: decoding the bytes that
`utf8EncodeBytes` produced reconstructs the original string, and any string
round-trips losslessly through the two functions.
[[src/builtins/encoding_package.mfb:__encoding_utf8EncodeBytes]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF Byte` | The UTF-8 octets to decode, one byte per element. Must form a well-formed UTF-8 sequence. [[src/builtins/encoding_package.mfb:__encoding_utf8DecodeBytes]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text; the empty string for an empty input list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | The bytes are not a well-formed UTF-8 sequence (invalid, overlong, or truncated). [[src/builtins/encoding_package.mfb:__encoding_utf8DecodeBytes]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Decode raw UTF-8 bytes back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("héllo")
  io::print(encoding::utf8DecodeBytes(raw))
END SUB
```

Round-trip a string through its UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("hi")
  io::print(encoding::utf8DecodeBytes(raw))
END SUB
```

## See also

- `mfb man encoding utf8Decode`
- `mfb man encoding utf8EncodeBytes`
- `mfb man encoding utf16Decode`
- `mfb man encoding hexDecode`
- `mfb man strings toBytes`
- `mfb man encoding`
