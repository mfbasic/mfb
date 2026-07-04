# utf8Decode

Decode a UTF-8 byte or code-unit sequence to a `String`.

## Synopsis

```
encoding::utf8Decode(value AS List OF Byte) AS String
encoding::utf8Decode(value AS List OF Integer) AS String
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

`encoding::utf8Decode` interprets `value` as a UTF-8 byte sequence and returns the
corresponding text. Because MFBASIC strings are always well-formed UTF-8, the
input is validated in full before the string is produced: `utf8Decode` accepts
only a well-formed UTF-8 sequence, rejecting an out-of-range element, an invalid
or overlong byte sequence, and any other malformed input. The empty list decodes
to the empty string. [[src/builtins/encoding_package.mfb:__encoding_utf8DecodeBytes]]

`utf8Decode` is a **parameter overload** selected by the argument's element type:
a `List OF Byte` is decoded directly, while a `List OF Integer` is first checked
element by element — each unit must lie in `0..255` — then decoded. The overload
is resolved during monomorphization, so the selection is a compile-time decision,
not a runtime dispatch. [[src/builtins/encoding.rs:resolve_overload_target]]

It is the inverse of `encoding::utf8Encode`: decoding the bytes (or integers)
that `utf8Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions.
[[src/builtins/encoding_package.mfb:__encoding_utf8DecodeInts]]

## Overloads

**`encoding::utf8Decode(value AS List OF Byte) AS String`**

Validates the raw octets as UTF-8 and returns the decoded text. Selected when the
argument is a `List OF Byte`. [[src/builtins/encoding.rs:UTF8_DECODE_BYTES]]

**`encoding::utf8Decode(value AS List OF Integer) AS String`**

Requires every element to be in `0..255`, then validates and decodes the resulting
bytes as UTF-8. Selected when the argument is a `List OF Integer`.
[[src/builtins/encoding.rs:UTF8_DECODE_INTS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF Byte` or `List OF Integer` | The UTF-8 bytes to decode. For the `List OF Integer` form, every element must be in the range `0..255`. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text; the empty string for an empty input list. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | An element is outside `0..255` (integer form), or the bytes are not a well-formed UTF-8 sequence. [[src/builtins/encoding_package.mfb:__encoding_utf8DecodeInts]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Type checking

`utf8Decode` takes exactly one argument, either a `List OF Byte` or a
`List OF Integer`, and returns a `String`. The argument type selects the overload
at compile time; any other argument type is a compile-time error.
[[src/builtins/encoding.rs:resolve_overload_target]] [[src/builtins/encoding.rs:arity]]

## Examples

Decode raw UTF-8 bytes back to text:

```
IMPORT encoding

LET raw AS List OF Byte = encoding::utf8Encode("héllo")
io::print(encoding::utf8Decode(raw))
```

Decode from a `List OF Integer` code-unit list:

```
IMPORT encoding

LET units AS List OF Integer = [104, 105]
io::print(encoding::utf8Decode(units))
```

## See also

- `mfb man encoding utf8Encode`
- `mfb man encoding utf16Decode`
- `mfb man encoding hexDecode`
- `mfb man strings toBytes`
- `mfb man encoding`
