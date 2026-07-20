# startsWith

Test whether a string begins with a given prefix.

## Synopsis

```
strings::startsWith(value AS String, prefix AS String) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::startsWith` returns `TRUE` when `value` begins with `prefix` and
`FALSE` otherwise. The test is an exact byte comparison of the leading bytes of
`value` against every byte of `prefix`, in order; it succeeds only when all of
them match. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_starts_with]] [[src/target/shared/code/builder_strings_package.rs:lower_string_prefix_predicate]]

No normalization, case folding, or other transformation is applied to either
operand, so `startsWith("Hello", "hello")` is `FALSE`. Because both operands are
well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte prefix is
always also a whole-scalar prefix — a match can never land mid-scalar.

The boundary cases follow from the byte comparison. A `prefix` longer than
`value` cannot match and returns `FALSE`. The empty `prefix` matches every
`value`, including the empty string, and returns `TRUE`. A non-empty `prefix`
against an empty `value` returns `FALSE`. Neither operand is modified and the
call never fails.

To test the end of the string use `strings::endsWith`; to test several candidate
prefixes at once use `strings::startsWithAny`; to remove the prefix rather than
test for it use `strings::stripPrefix`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose leading bytes are examined. May be empty. [[src/builtins/strings.rs:call_param_names]] |
| `prefix` | `String` | The prefix to look for at the start of `value`. May be empty, in which case the result is always `TRUE`. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the bytes of `prefix` match the leading bytes of `value`, `FALSE` otherwise. An empty `prefix` always yields `TRUE`; a `prefix` longer than `value` always yields `FALSE`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Test for a leading prefix, including a multi-byte one:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::startsWith("Hello", "He")))
  io::print(toString(strings::startsWith("😀 Hello", "😀")))
  io::print(toString(strings::startsWith("Hello", "hello")))
  RETURN 0
END FUNC
```

The empty prefix always matches:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::startsWith("anything", "")))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings endsWith`
- `mfb man strings startsWithAny`
- `mfb man strings stripPrefix`
- `mfb man strings contains`
- `mfb man strings`
