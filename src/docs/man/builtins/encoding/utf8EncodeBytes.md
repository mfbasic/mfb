# utf8EncodeBytes

Encode a `String` to its UTF-8 bytes as a `List OF Byte`.

## Synopsis

```
encoding::utf8EncodeBytes(value AS String) AS List OF Byte
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

`encoding::utf8EncodeBytes` returns the UTF-8 encoding of `value` — the exact
bytes that make up the string's storage — as a `List OF Byte`, one element per
byte. Because MFBASIC strings are always UTF-8 text, the result is the string's
raw octets in order, each element in the range `0..255`. The result is exactly
`strings::toBytes(value)`. [[src/builtins/encoding_package.mfb:__encoding_utf8EncodeBytes]]

This is the byte-typed form of `encoding::utf8Encode`. `utf8Encode` is a
return-type overload that selects between `List OF Byte` and `List OF Integer`
from the call's contextual type; `utf8EncodeBytes` is the concrete, non-overloaded
name that always yields `List OF Byte`, so no type context is needed to
disambiguate it. The integer-typed counterpart is `encoding::utf8EncodeInts`.
[[src/builtins/encoding.rs:UTF8_ENCODE_BYTES]] [[src/builtins/encoding.rs:resolve_overload_target]]

The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error.

The inverse operation is `encoding::utf8DecodeBytes`, which accepts a
`List OF Byte` and validates it as well-formed UTF-8.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to encode. Any string, including the empty string, is accepted. [[src/builtins/encoding_package.mfb:__encoding_utf8EncodeBytes]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The UTF-8 bytes of `value`, one element per byte (`0..255`); empty for the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a string to raw UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("héllo")
  io::print(toString(len(raw)))
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

- `mfb man encoding utf8Encode`
- `mfb man encoding utf8Decode`
- `mfb man strings toBytes`
- `mfb man encoding`
