# toBytes

Return the raw UTF-8 bytes backing a string, one element per byte.

## Synopsis

```
strings::toBytes(value AS String) AS List OF Byte
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::toBytes` returns the UTF-8 octets that back `value` as a
`List OF Byte`, one element per byte, in encoding order. It is the byte-level
view of a string: no decoding, validation, or transformation is performed, and
the bytes are copied verbatim into a freshly built list.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_to_bytes]]

The result length is exactly `strings::byteLen(value)`, which is generally larger
than `len(value)`: an ASCII scalar contributes one element, while a non-ASCII
scalar contributes the two, three, or four bytes of its UTF-8 encoding. For
`"héllo"` the list has six elements, because `é` encodes as the two bytes `195`
and `169`. The empty string yields the empty list.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_byte_len]]

`toBytes` is the inverse of `toString(List OF Byte)` and the foundation the
`encoding` package's Unicode codecs are built on; `encoding::utf8EncodeBytes`
produces the same octets for the same string. When `value` is a compile-time
constant, the list is folded at build time from the literal's bytes rather than
built at run time — the observable result is identical.
[[src/target/shared/code/builder_strings_package.rs:lower_strings_package_call]]

`value` is not mutated. The returned `List OF Byte` is a fresh owned value, so
mutating it does not affect the string it came from.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose UTF-8 storage is returned. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The UTF-8 octets of `value` in order, one per element; the empty list for the empty string. The element count equals `strings::byteLen(value)`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

A non-ASCII scalar contributes more than one byte:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  LET bytes AS List OF Byte = strings::toBytes("héllo")
  io::print(toString(len(bytes)))
  io::print(toString(collections::get(bytes, 1)))
  RETURN 0
END FUNC
```

Round-trip a string through its bytes:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET bytes AS List OF Byte = strings::toBytes("hi")
  io::print(toString(bytes))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings byteLen`
- `mfb man strings toScalars`
- `mfb man encoding utf8EncodeBytes`
- `mfb man encoding hexEncode`
- `mfb man general toString`
- `mfb man strings`
