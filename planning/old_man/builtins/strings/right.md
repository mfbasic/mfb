# right

Return the trailing Unicode scalars of a string.

## Synopsis

```
strings::right(value AS String, count AS Integer) AS String
strings::right(value AS AttributedString, count AS Integer) AS AttributedString
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/codegen/registry/mod.rs:is_member]]

## Description

`strings::right` returns a new `String` holding the last `count` Unicode scalar
values of `value`. The selected scalars keep their original left-to-right order;
`right` does not reverse them.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_left_right]]

Lengths are measured in Unicode scalar values — not UTF-8 bytes and not grapheme
clusters. A multi-byte scalar such as `é` or `😀` counts as one even though it
occupies several bytes, and `right` never splits a scalar, so the result is
always well-formed UTF-8. Note that a grapheme cluster made of a base scalar plus
combining marks counts as more than one, so `right` can cut a cluster in half;
use `strings::graphemes` when user-perceived characters are what matters.

`right` clamps rather than failing on an over-long request: when `count` is
greater than or equal to the scalar length of `value`, the whole string is
returned, with no padding and no error. A `count` of `0` returns the empty
string. A negative `count` is rejected with `ErrInvalidArgument`.

This clamping is the difference from `strings::mid`, which raises
`ErrIndexOutOfRange` when the requested window runs past the end.

`value` is not mutated; the result is a new owned `String`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit.
[[src/codegen/builtins/strings/mod.rs:is_tier_b_transform]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose trailing scalars are returned. May be empty. [[src/codegen/registry/mod.rs:call_param_names]] |
| `count` | `Integer` | The number of trailing Unicode scalar values to take. Must be `0` or greater; values at or above the scalar length of `value` yield the whole string. [[src/codegen/registry/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding the last `count` scalars of `value`, in original order. `""` when `count` is `0`; the whole of `value` when `count` is at least its scalar length. [[src/codegen/builtins/strings/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `count` is negative. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_left_right]] [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidArgument]] |

## Examples

Take a suffix; an over-long count clamps to the whole string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::right("hello", 3))
  io::print(strings::right("hi", 5))
  io::print("[" & strings::right("hi", 0) & "]")
  RETURN 0
END FUNC
```

Multi-byte scalars count as one position each:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::right("ab😀c", 2))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings left`
- `mfb man strings mid`
- `mfb man strings padRight`
- `mfb man general len`
- `mfb man strings`
