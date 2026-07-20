# byteLen

Return the UTF-8 byte length of a string.

## Synopsis

```
strings::byteLen(value AS String) AS Integer
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::byteLen` returns the number of bytes `value` occupies in its UTF-8
encoding. It measures storage size, not character count: every byte of the
encoding is counted exactly once. The length is read directly from the string's
stored byte count, so the call is constant time and does not scan the text.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_byte_len]]

Because UTF-8 uses a variable number of bytes per Unicode scalar value, the
result can exceed `len(value)`, which counts Unicode scalar values. ASCII scalars
occupy one byte each, so the two counts are equal for pure-ASCII text; scalars
outside ASCII occupy two, three, or four bytes each, making the byte length
larger. `byteLen` is therefore always greater than or equal to `len(value)`.
[[src/target/shared/code/builder_collection_layout.rs:lower_len]]

The empty string has a byte length of `0`. `byteLen` inspects `value` only: it
allocates nothing, mutates nothing, and is locale-independent.

To count Unicode scalar values use the bare `len` builtin; to count
user-perceived characters use `strings::graphemesCount`; to obtain the individual
bytes use `strings::toBytes`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to measure. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of bytes in the UTF-8 encoding of `value`; `0` for the empty string. Always greater than or equal to `len(value)`, and equal to it exactly when `value` is pure ASCII. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

An ASCII string has one byte per scalar, a non-ASCII one does not:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::byteLen("Hello")))
  io::print(toString(strings::byteLen("😀")))
  RETURN 0
END FUNC
```

Compare byte length with scalar count:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::byteLen("A😀é")))
  io::print(toString(len("A😀é")))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings toBytes`
- `mfb man strings graphemesCount`
- `mfb man general len`
- `mfb man unicode`
- `mfb man strings`
