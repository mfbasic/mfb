# utf8DecodeInts

Decode a `List OF Integer` of UTF-8 code units to a `String`.

## Synopsis

```
encoding::utf8DecodeInts(value AS List OF Integer) AS String
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

`encoding::utf8DecodeInts` interprets `value` as a UTF-8 byte sequence held one
octet per integer element and returns the corresponding text. Each element is
first range-checked and narrowed to a byte: every unit must lie in `0..255`
(0 through 255 inclusive), and the assembled bytes must form a well-formed UTF-8
sequence, with no invalid, overlong, or truncated byte sequence. If both checks
pass, the octets become the string's storage. The empty list decodes to the
empty string. [[src/builtins/encoding_package.mfb:__encoding_utf8DecodeInts]]

This is the integer-typed form of `encoding::utf8Decode`. `utf8Decode` is a
parameter overload that selects between a `List OF Byte` and a `List OF Integer`
argument at compile time; `utf8DecodeInts` is the concrete, non-overloaded name
that always takes a `List OF Integer`, so no overload resolution is involved.
The byte-typed counterpart is `encoding::utf8DecodeBytes`, which takes a
`List OF Byte` and therefore performs no per-element range check.
[[src/builtins/encoding.rs:UTF8_DECODE_INTS]] [[src/builtins/encoding.rs:resolve_overload_target]]

It is the inverse of `encoding::utf8EncodeInts`: decoding the integers that
`utf8EncodeInts` produced reconstructs the original string, and any string
round-trips losslessly through the two functions.
[[src/builtins/encoding_package.mfb:__encoding_utf8EncodeInts]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF Integer` | The UTF-8 octets to decode, one code unit per element. Each element must be in `0..255` and the sequence must be well-formed UTF-8. [[src/builtins/encoding_package.mfb:__encoding_utf8DecodeInts]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text; the empty string for an empty input list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | An element is outside `0..255`, or the assembled bytes are not a well-formed UTF-8 sequence (invalid, overlong, or truncated). [[src/builtins/encoding_package.mfb:__encoding_utf8DecodeInts]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Decode UTF-8 code units back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("héllo")
  io::print(encoding::utf8DecodeInts(units))
END SUB
```

Round-trip a string through its UTF-8 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("hi")
  io::print(encoding::utf8DecodeInts(units))
END SUB
```

## See also

- `mfb man encoding utf8Decode`
- `mfb man encoding utf8DecodeBytes`
- `mfb man encoding utf8EncodeInts`
- `mfb man encoding utf16Decode`
- `mfb man encoding`
