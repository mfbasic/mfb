# repeat

Concatenate a string with itself a given number of times.

## Synopsis

```
strings::repeat(value AS String, times AS Integer) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::repeat` returns a new `String` made of `times` consecutive copies of
`value`, written end to end with nothing inserted between them.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_repeat]]

Copying works on the raw UTF-8 bytes of `value`, so every multi-byte scalar and
every grapheme cluster is reproduced intact in each copy — `repeat` never splits
a character. The byte length of the result is exactly
`strings::byteLen(value) * times`.

A `times` of `0` returns the empty string regardless of `value`, and a `times` of
`1` returns a copy equal to `value`. Repeating the empty string yields the empty
string for any valid `times`. A negative `times` is rejected with
`ErrInvalidArgument`.

The total size is computed with overflow checks. A `byteLen(value) * times`
product, or the string header added to it, that cannot be represented in 64 bits
raises the same `ErrInvalidArgument` rather than allocating short and writing
past the buffer. [[src/target/shared/code/builder_error_emission.rs:emit_checked_size_multiply]]

`value` is not mutated; the result is a new owned `String`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to repeat. Any `String`, including the empty one. [[src/builtins/strings.rs:call_param_names]] |
| `times` | `Integer` | The number of copies to concatenate. Must be `0` or greater. `0` yields `""` and `1` yields a copy of `value`. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding `times` consecutive copies of `value`. `""` when `times` is `0` or when `value` is empty. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `times` is negative, or the result size (`byteLen(value) * times`, plus the string header) cannot be represented in 64 bits. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_repeat]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Repeat a short string; zero copies yields the empty string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::repeat("ab", 3))
  io::print("[" & strings::repeat("x", 0) & "]")
  RETURN 0
END FUNC
```

Build a horizontal rule:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::repeat("-", 40))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings join`
- `mfb man strings padLeft`
- `mfb man strings padRight`
- `mfb man strings byteLen`
- `mfb man strings`
