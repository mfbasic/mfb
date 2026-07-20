# endsWith

Test whether a string ends with a given suffix.

## Synopsis

```
strings::endsWith(value AS String, suffix AS String) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::endsWith` returns `TRUE` when `value` ends with `suffix` and `FALSE`
otherwise. The test is an exact byte comparison of the trailing bytes of `value`
against every byte of `suffix`, in order; it succeeds only when all of them
match. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_ends_with]] [[src/target/shared/code/builder_strings_package.rs:lower_string_prefix_predicate]]

No normalization, case folding, or other transformation is applied to either
operand, so `endsWith("Hello", "LO")` is `FALSE`. Because both operands are
well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte suffix is
always also a whole-scalar suffix — a match can never land mid-scalar.

The boundary cases follow from the byte comparison. A `suffix` longer than
`value` cannot match and returns `FALSE`. The empty `suffix` matches every
`value`, including the empty string, and returns `TRUE`. A non-empty `suffix`
against an empty `value` returns `FALSE`. Neither operand is modified and the
call never fails.

To test the start of the string use `strings::startsWith`; to test several
candidate suffixes at once use `strings::endsWithAny`; to remove the suffix
rather than test for it use `strings::stripSuffix`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose trailing bytes are examined. May be empty. [[src/builtins/strings.rs:call_param_names]] |
| `suffix` | `String` | The suffix to look for at the end of `value`. May be empty, in which case the result is always `TRUE`. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the bytes of `suffix` match the trailing bytes of `value`, `FALSE` otherwise. An empty `suffix` always yields `TRUE`; a `suffix` longer than `value` always yields `FALSE`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Test for a trailing suffix, including a multi-byte one:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::endsWith("Hello", "lo")))
  io::print(toString(strings::endsWith("Hello 😀", "😀")))
  io::print(toString(strings::endsWith("Hi", "Hello")))
  RETURN 0
END FUNC
```

Match a file extension:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::endsWith("photo.png", ".png")))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings startsWith`
- `mfb man strings endsWithAny`
- `mfb man strings stripSuffix`
- `mfb man strings contains`
- `mfb man strings`
