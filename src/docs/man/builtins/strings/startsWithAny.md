# startsWithAny

Test whether a string begins with any of several prefixes.

## Synopsis

```
strings::startsWithAny(value AS String, prefixes AS List OF String) AS Boolean
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::startsWithAny` returns `TRUE` when `value` begins with at least one of
the strings in `prefixes`, and `FALSE` otherwise. Candidates are tested in list
order and the scan stops at the first match; which candidate matched is not
reported, only that one did. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_with_any]]

Each individual test is the same exact byte comparison `strings::startsWith`
performs: the leading bytes of `value` must equal every byte of the candidate, in
order, with no normalization and no case folding. A candidate longer than `value`
cannot match and is skipped rather than treated as an error.
[[src/target/shared/code/private/unicode.rs:emit_string_byte_range_equal_branch]]

An empty string appearing as a candidate matches everything, so a `prefixes` list
containing `""` makes the result `TRUE` for any `value`. An empty `prefixes` list
has no candidates and returns `FALSE`. Neither `value` nor the list is modified,
and the call is total — it never fails.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose leading bytes are examined. May be empty. [[src/builtins/strings.rs:call_param_names]] |
| `prefixes` | `List OF String` | The candidate prefixes, tested in list order. May be empty, in which case the result is `FALSE`. Entries may themselves be empty; an empty entry always matches. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when some entry of `prefixes` is a prefix of `value`, `FALSE` otherwise. An empty list yields `FALSE`; a list containing `""` always yields `TRUE`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Test a URL against several scheme prefixes:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET schemes AS List OF String = ["http://", "https://"]
  io::print(toString(strings::startsWithAny("https://example.com", schemes)))
  RETURN 0
END FUNC
```

An empty candidate list never matches:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET none AS List OF String = []
  io::print(toString(strings::startsWithAny("anything", none)))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings startsWith`
- `mfb man strings endsWithAny`
- `mfb man strings stripPrefix`
- `mfb man strings contains`
- `mfb man strings`
