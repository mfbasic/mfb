# split

Split a string into a list of substrings around a delimiter.

## Synopsis

```
strings::split(value AS String, delimiter AS String) AS List OF String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::split` scans `value` left to right, breaks it at every non-overlapping
occurrence of `delimiter`, and returns the pieces between the matches as a
`List OF String`. The delimiters themselves are removed from the output.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_split]]

Matching is an exact byte comparison with no normalization and no case folding.
After a match is consumed, scanning resumes at the byte immediately following the
matched delimiter, so matches never overlap. Because both operands are
well-formed UTF-8, a delimiter is only found where its complete byte sequence
appears, so a split can never land mid-scalar.
[[src/target/shared/code/builder_strings_package.rs:emit_string_split_write_entry]]

The result always contains exactly one more element than the number of matches
found, and is therefore never empty. Everything else follows from that rule:

- A `delimiter` that does not occur — including one longer than `value` — yields
  a single-element list holding `value` unchanged.
- A leading match yields a leading empty element; a trailing match yields a
  trailing empty element.
- Two adjacent matches yield an empty element between them, so
  `split(",a,,", ",")` has four elements: `""`, `"a"`, `""`, `""`.
- Splitting the empty string yields a single-element list holding `""`.

`delimiter` must not be empty; an empty delimiter is rejected with
`ErrInvalidArgument` before any scanning occurs. `value` is not mutated; the
returned list and its elements are fresh owned values.

`delimiter` is also accepted under the name `separator`. Joining the result with
the same non-empty delimiter reproduces `value` exactly — `split` and
`strings::join` are inverses. [[src/builtins/strings.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to divide. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |
| `delimiter` | `String` | The separator to break `value` on. Must be non-empty. Also accepted under the name `separator`. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF String` | The substrings lying between successive matches, in left-to-right order. Length is always the match count plus one, so the list is never empty; with no match it holds `value` as its single element. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `delimiter` is the empty string. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_split]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Split a comma-separated line:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  LET parts AS List OF String = strings::split("a,b,c", ",")
  io::print(toString(len(parts)))
  io::print(collections::get(parts, 1))
  RETURN 0
END FUNC
```

Empty fields at the edges are preserved, and a delimiter that does not occur
returns one element:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(len(strings::split(",a,,", ","))))
  io::print(toString(len(strings::split("abc", "|"))))
  RETURN 0
END FUNC
```

Split and rejoin to round-trip the value:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET parts AS List OF String = strings::split("a😀b😀c", "😀")
  io::print(strings::join(parts, "-"))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings join`
- `mfb man strings count`
- `mfb man strings find`
- `mfb man strings replace`
- `mfb man strings`
