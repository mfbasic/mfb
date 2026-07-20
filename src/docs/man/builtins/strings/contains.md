# contains

Test whether a string contains another string.

## Synopsis

```
strings::contains(value AS String, needle AS String) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::contains` returns `TRUE` when `needle` occurs as a contiguous
substring of `value`, and `FALSE` otherwise. The bytes of `needle` are compared
against `value` at each successive byte offset, and the scan returns `TRUE` at
the first offset where the whole needle matches.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_contains]] [[src/target/shared/code/private/unicode.rs:emit_string_byte_range_equal_branch]]

No normalization, case folding, or other transformation is applied to either
operand. Because both are well-formed UTF-8 and UTF-8 is self-synchronizing, a
matching byte run is always also a whole-scalar substring — a match can never
land mid-scalar.

The empty `needle` occurs at every position and returns `TRUE` for any `value`,
including the empty string. A `needle` longer than `value` returns `FALSE`, and
searching a non-empty `needle` in an empty `value` returns `FALSE`. Neither
operand is modified and the call never fails.

`contains` answers only *whether* the needle is present. Use `strings::find` to
get the position of the first occurrence — and note that `find` raises
`ErrNotFound` on absence, so guarding it with `contains` is the idiomatic way to
treat absence as an ordinary outcome. Use `strings::count` for the number of
occurrences.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to search within. May be empty. [[src/builtins/strings.rs:call_param_names]] |
| `needle` | `String` | The substring to look for anywhere in `value`. May be empty, in which case the result is always `TRUE`. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `needle` occurs contiguously within `value`, `FALSE` otherwise. An empty `needle` always yields `TRUE`; a `needle` longer than `value` always yields `FALSE`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Test for a substring, including a multi-byte one:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::contains("Hello", "ell")))
  io::print(toString(strings::contains("Hello 😀", "😀")))
  io::print(toString(strings::contains("Hello", "xyz")))
  RETURN 0
END FUNC
```

Guard `find` with `contains` when absence is expected:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET text AS String = "hello world"
  IF strings::contains(text, "world") THEN
    io::print(toString(strings::find(text, "world")))
  ELSE
    io::print("absent")
  END IF
  RETURN 0
END FUNC
```

## See also

- `mfb man strings find`
- `mfb man strings count`
- `mfb man strings startsWith`
- `mfb man strings endsWith`
- `mfb man strings`
