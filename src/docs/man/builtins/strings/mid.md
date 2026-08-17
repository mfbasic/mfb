# mid

Extract a substring by Unicode scalar index and length.

## Synopsis

```
strings::mid(value AS String, start AS Integer, count AS Integer) AS String
strings::mid(value AS AttributedString, start AS Integer, count AS Integer) AS AttributedString
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/codegen/registry/mod.rs:is_member]]

## Description

`strings::mid` returns a new `String` holding `count` Unicode scalar values of
`value`, beginning at the zero-based scalar position `start`.
[[src/target/shared/code/builder_search.rs:lower_mid]]

Positions and lengths are measured in Unicode scalar values — not UTF-8 bytes and
not grapheme clusters. A multi-byte scalar such as `é` or `😀` counts as one
position even though it occupies several bytes, and `mid` never splits a scalar,
so the returned string is always well-formed UTF-8. A grapheme cluster made of a
base scalar plus combining marks counts as more than one position, so `mid` can
cut a cluster in half; use `strings::graphemes` when user-perceived characters
are what matters. [[src/target/shared/code/private/unicode.rs:emit_utf8_decode_next]]

`start` is the index of the first scalar to include and `count` is how many to
take; both must be `0` or greater. A `count` of `0` returns the empty string,
even at the very end of `value`, and `start` may equal the scalar length of
`value`, which with a `count` of `0` selects the empty slice at the end.

Unlike `strings::left` and `strings::right`, `mid` does **not** clamp. It never
silently shortens a window that runs past the end: `start` must not exceed the
scalar length of `value`, and `start + count` must not exceed it either.
Violating that raises `ErrIndexOutOfRange`, which makes an over-long request a
detectable mistake rather than a truncated result. A `start + count` sum that
overflows 64 bits raises the same error.

`value` is not mutated; the result is a new owned `String`. The bare `mid` name
is also defined for lists; see `mfb man collections mid`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is the same slice as the `String` overload's and
whose attribute spans are remapped by the same edit. [[src/codegen/builtins/strings/mod.rs:is_tier_b_transform]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to slice. May be empty. [[src/codegen/registry/mod.rs:call_param_names]] |
| `start` | `Integer` | The zero-based scalar index of the first scalar to include. Must be in `0` through the scalar length of `value` inclusive. [[src/codegen/registry/mod.rs:call_param_names]] |
| `count` | `Integer` | The number of Unicode scalar values to take. Must be `0` or greater, and `start + count` must not exceed the scalar length of `value`. [[src/codegen/registry/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding `count` scalars of `value` starting at `start`. `""` when `count` is `0`. [[src/codegen/builtins/strings/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `start` is negative, `count` is negative, `start` exceeds the scalar length of `value`, `start + count` exceeds it, or `start + count` overflows 64 bits. [[src/target/shared/code/builder_search.rs:lower_mid]] [[src/codegen/builtins/errorcode/mod.rs:ErrIndexOutOfRange]] |

## Examples

Slice from the middle; a zero count yields the empty string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::mid("hello", 1, 3))
  io::print("[" & strings::mid("hello", 5, 0) & "]")
  RETURN 0
END FUNC
```

Multi-byte scalars count as single positions:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::mid("a😀é日", 1, 2))
  RETURN 0
END FUNC
```

Catch an over-long window instead of getting a truncated result:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET part AS String = strings::mid("hello", 3, 3) TRAP(e)
    io::print("out of range")
    RETURN 0
  END TRAP
  io::print(part)
  RETURN 0
END FUNC
```

## See also

- `mfb man strings left`
- `mfb man strings right`
- `mfb man strings find`
- `mfb man general len`
- `mfb man collections mid`
- `mfb man strings`
