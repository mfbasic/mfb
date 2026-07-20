# utf8EncodeInts

Encode a `String` to its UTF-8 bytes as a `List OF Integer`.

## Synopsis

```
encoding::utf8EncodeInts(value AS String) AS List OF Integer
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

`encoding::utf8EncodeInts` returns the UTF-8 encoding of `value` — the exact
bytes that make up the string's storage — as a `List OF Integer`, one element per
byte. Because MFBASIC strings are always UTF-8 text, the result is the string's
raw octets in order, each element widened to `Integer` and in the range `0..255`.
The elements are exactly the values of `strings::toBytes(value)` converted with
`toInt`. [[src/builtins/encoding_package.mfb:__encoding_utf8EncodeInts]]

This is the integer-typed form of `encoding::utf8Encode`. `utf8Encode` is a
return-type overload that selects between `List OF Byte` and `List OF Integer`
from the call's contextual type; `utf8EncodeInts` is the concrete, non-overloaded
name that always yields `List OF Integer`, so no type context is needed to
disambiguate it. The byte-typed counterpart is `encoding::utf8EncodeBytes`.
[[src/builtins/encoding.rs:UTF8_ENCODE_INTS]] [[src/builtins/encoding.rs:resolve_overload_target]]

The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error.

The inverse operation is `encoding::utf8DecodeInts`, which accepts a
`List OF Integer` and validates it as well-formed UTF-8.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to encode. Any string, including the empty string, is accepted. [[src/builtins/encoding_package.mfb:__encoding_utf8EncodeInts]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Integer` | The UTF-8 bytes of `value` as `Integer` elements, one per byte (`0..255`); empty for the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a string to its UTF-8 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("héllo")
  io::print(toString(len(units)))
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

- `mfb man encoding utf8Encode`
- `mfb man encoding utf8EncodeBytes`
- `mfb man encoding utf8Decode`
- `mfb man strings toBytes`
- `mfb man encoding`
