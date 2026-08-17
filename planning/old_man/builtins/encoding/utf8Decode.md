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
dependency is required. [[src/codegen/builtins/encoding/mod.rs:augmented_project]]

## Description

`encoding::utf8Decode` interprets `value` as a UTF-8 byte sequence and returns the
corresponding text. Because MFBASIC strings are always well-formed UTF-8, the
input is validated in full before the string is produced: `utf8Decode` accepts
only a well-formed UTF-8 sequence and rejects an invalid lead byte, a missing or
stray continuation byte, a truncated multi-byte sequence, an overlong encoding, a
surrogate code point (`U+D800`–`U+DFFF`), and any scalar above `U+10FFFF`. The
empty list decodes to the empty string.
[[src/codegen/builtins/encoding/package.mfb:__encoding_utf8Valid]] [[src/codegen/builtins/encoding/package.mfb:__encoding_utf8Decode]]

`utf8Decode` is a **parameter overload** selected by the argument's element type:
a `List OF Byte` is decoded directly, while a `List OF Integer` is first checked
element by element — each unit must lie in `0..255` — then decoded. The overload
is resolved during monomorphization, so the selection is a compile-time decision,
not a runtime dispatch. [[src/monomorph/lower.rs:resolve_overload]]

It is the inverse of `encoding::utf8Encode`: decoding the bytes (or integers)
that `utf8Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions.
[[src/codegen/builtins/encoding/package.mfb:__encoding_utf8Decode]]

## Overloads

**`encoding::utf8Decode(value AS List OF Byte) AS String`**

Validates the raw octets as UTF-8 and returns the decoded text. Selected when the
argument is a `List OF Byte`. [[src/codegen/builtins/encoding/package.mfb:__encoding_utf8Decode]]

**`encoding::utf8Decode(value AS List OF Integer) AS String`**

Requires every element to be in `0..255`, then validates and decodes the resulting
bytes as UTF-8. Selected when the argument is a `List OF Integer`.
[[src/codegen/builtins/encoding/package.mfb:__encoding_utf8Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF Byte` or `List OF Integer` | The UTF-8 bytes to decode. For the `List OF Integer` form, every element must be in the range `0..255`. [[src/codegen/registry/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded text; the empty string for an empty input list. [[src/codegen/builtins/encoding/func_utf8_decode.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | An element is outside `0..255` (integer form), or the bytes are not a well-formed UTF-8 sequence. [[src/codegen/builtins/encoding/package.mfb:__encoding_utf8Decode]] [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidFormat]] |

## Type checking

`utf8Decode` takes exactly one argument, either a `List OF Byte` or a
`List OF Integer`, and returns a `String`. The argument type selects the overload
at compile time; any other argument type is a compile-time error.
[[src/monomorph/lower.rs:resolve_overload]] [[src/codegen/builtins/encoding/func_utf8_decode.rs:register]]

## Examples

Decode raw UTF-8 bytes back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(encoding::utf8Decode(raw))
END SUB
```

Decode from a `List OF Integer` code-unit list:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = [104, 105]
  io::print(encoding::utf8Decode(units))
END SUB
```

## See also

- `mfb man encoding utf8Encode`
- `mfb man encoding utf16Decode`
- `mfb man encoding hexDecode`
- `mfb man strings toBytes`
- `mfb man encoding`
