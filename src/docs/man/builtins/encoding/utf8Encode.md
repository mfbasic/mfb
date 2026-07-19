# utf8Encode

Encode a `String` to its UTF-8 bytes.

## Synopsis

```
encoding::utf8Encode(value AS String) AS List OF Byte
encoding::utf8Encode(value AS String) AS List OF Integer
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

`encoding::utf8Encode` returns the UTF-8 encoding of `value` — the exact bytes
that make up the string's storage — one element per byte. Because MFBASIC strings
are always UTF-8 text, the result is the string's raw octets in order, with each
element in the range `0..255`. [[src/builtins/encoding_package.mfb:__encoding_utf8EncodeBytes]]

The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error. The
byte form is exactly `strings::toBytes(value)`; the integer form contains the
identical numeric values widened to `Integer`. [[src/builtins/encoding_package.mfb:__encoding_utf8EncodeInts]]

`utf8Encode` is a **return-type overload**: the same `String` argument produces
either a `List OF Byte` or a `List OF Integer`, chosen by the expected
(contextual) type at the call site. A call with no type context to select the
overload is a compile-time `TYPE_OVERLOAD_AMBIGUOUS` error, not a runtime failure;
the overload is resolved during monomorphization. [[src/builtins/encoding.rs:resolve_overload_target]]

The inverse operation is `encoding::utf8Decode`, which accepts either a
`List OF Byte` or a `List OF Integer` and validates it as well-formed UTF-8.

## Overloads

**`encoding::utf8Encode(value AS String) AS List OF Byte`**

Returns the UTF-8 octets as raw bytes. Selected when the call is used where a
`List OF Byte` is expected. [[src/builtins/encoding.rs:UTF8_ENCODE_BYTES]]

**`encoding::utf8Encode(value AS String) AS List OF Integer`**

Returns the identical byte values as `Integer` elements (each `0..255`). Selected
when the call is used where a `List OF Integer` is expected. [[src/builtins/encoding.rs:UTF8_ENCODE_INTS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The text to encode. Any string, including the empty string, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The UTF-8 bytes of `value`, one element per byte (`0..255`); empty for the empty string. [[src/builtins/encoding.rs:call_return_type_name]] |
| `List OF Integer` | The same UTF-8 byte values as `Integer` elements (`0..255`). [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Type checking

`utf8Encode` takes exactly one `String` argument. The return type is a return-type
overload resolved to `List OF Byte` or `List OF Integer` from the expected type;
with no expected type to disambiguate, the call is a compile-time
`TYPE_OVERLOAD_AMBIGUOUS` error. [[src/builtins/encoding.rs:resolve_overload_target]] [[src/builtins/encoding.rs:arity]]

## Examples

Encode a string to raw UTF-8 bytes:

```
IMPORT encoding

LET raw AS List OF Byte = encoding::utf8Encode("héllo")
io::print(toString(len(raw)))
```

Encode to the `List OF Integer` form and round-trip it back:

```
IMPORT encoding

LET units AS List OF Integer = encoding::utf8Encode("hi")
io::print(encoding::utf8Decode(units))
```

## See also

- `mfb man encoding utf8EncodeBytes`
- `mfb man encoding utf8EncodeInts`
- `mfb man encoding utf8Decode`
- `mfb man encoding utf16Encode`
- `mfb man encoding hexEncode`
- `mfb man strings toBytes`
- `mfb man encoding`
