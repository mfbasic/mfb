# endsWithAny

Test whether a string ends with any of several suffixes.

## Synopsis

```
strings::endsWithAny(value AS String, suffixes AS List OF String) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::endsWithAny` returns `TRUE` when `value` ends with at least one of the
strings in `suffixes`, and `FALSE` otherwise. Candidates are tested in list order
and the scan stops at the first match; which candidate matched is not reported,
only that one did. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_with_any]]

Each individual test is the same exact byte comparison `strings::endsWith`
performs: the trailing bytes of `value` must equal every byte of the candidate,
in order, with no normalization and no case folding. A candidate longer than
`value` cannot match and is skipped rather than treated as an error.
[[src/target/shared/code/private/unicode.rs:emit_string_byte_range_equal_branch]]

An empty string appearing as a candidate matches everything, so a `suffixes` list
containing `""` makes the result `TRUE` for any `value`. An empty `suffixes` list
has no candidates and returns `FALSE`. Neither `value` nor the list is modified,
and the call is total — it never fails.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose trailing bytes are examined. May be empty. [[src/builtins/strings.rs:call_param_names]] |
| `suffixes` | `List OF String` | The candidate suffixes, tested in list order. May be empty, in which case the result is `FALSE`. Entries may themselves be empty; an empty entry always matches. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when some entry of `suffixes` is a suffix of `value`, `FALSE` otherwise. An empty list yields `FALSE`; a list containing `""` always yields `TRUE`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Test a filename against several image extensions:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET images AS List OF String = [".png", ".jpg", ".gif"]
  io::print(toString(strings::endsWithAny("photo.png", images)))
  RETURN 0
END FUNC
```

An empty candidate list never matches:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET none AS List OF String = []
  io::print(toString(strings::endsWithAny("anything", none)))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings endsWith`
- `mfb man strings startsWithAny`
- `mfb man strings stripSuffix`
- `mfb man strings contains`
- `mfb man strings`
