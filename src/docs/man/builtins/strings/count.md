# count

Count the non-overlapping occurrences of a substring.

## Synopsis

```
strings::count(value AS String, needle AS String) AS Integer
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::count` returns the number of non-overlapping occurrences of `needle`
within `value`. The scan starts at the first byte of `value` and compares the
bytes of `needle` at the current offset. On a match the count is incremented and
the cursor advances past the whole matched needle; on a mismatch the cursor
advances by a single byte. The scan ends once fewer than `byteLen(needle)` bytes
remain. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_count]]

The non-overlapping rule matters for self-similar needles. Counting `"aa"` in
`"aaa"` yields `1`, not `2`, because after the match at offset `0` the cursor
jumps to offset `2`. Counting `"a"` in `"aaa"` yields `3`.

Matching is an exact byte comparison with no normalization and no case folding.
Because both operands are well-formed UTF-8 and UTF-8 is self-synchronizing, a
multi-byte needle is only ever reported where its complete byte sequence appears,
so a match can never land mid-scalar.
[[src/target/shared/code/private/unicode.rs:emit_string_byte_range_equal_branch]]

A `needle` longer than `value` yields `0`, as does an empty `value`. The empty
`needle` has no well-defined occurrence count and is rejected with
`ErrInvalidArgument` — note that this differs from `strings::contains` and
`strings::find`, which both accept an empty needle. Neither operand is modified.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to scan. May be empty, in which case the result is `0`. [[src/builtins/strings.rs:call_param_names]] |
| `needle` | `String` | The substring to count. Must be non-empty. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of non-overlapping occurrences of `needle` in `value`, counted left to right. `0` when `needle` does not occur, when it is longer than `value`, or when `value` is empty. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `needle` is the empty string. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_count]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Count a repeated substring:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::count("abcabc", "bc")))
  io::print(toString(strings::count("xyz", "a")))
  RETURN 0
END FUNC
```

Matches never overlap:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::count("aaa", "a")))
  io::print(toString(strings::count("aaa", "aa")))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings contains`
- `mfb man strings find`
- `mfb man strings replace`
- `mfb man strings split`
- `mfb man strings`
