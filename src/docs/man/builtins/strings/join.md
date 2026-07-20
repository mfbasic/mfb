# join

Concatenate a list of strings, inserting a delimiter between elements.

## Synopsis

```
strings::join(parts AS List OF String, delimiter AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::join` walks `parts` from first to last and concatenates the elements
into one `String`, placing a single copy of `delimiter` between each adjacent
pair. The delimiter goes *between* elements only — never before the first and
never after the last — so a list of N elements produces exactly N − 1 delimiters.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_join]]

Concatenation copies the raw UTF-8 bytes of each element and of the delimiter
verbatim. No trimming, normalization, or case folding is performed. Placing one
well-formed UTF-8 fragment after another always yields well-formed UTF-8, so the
result is a valid string.

`delimiter` may be any `String`, including the empty one, which concatenates the
elements with nothing between them. Elements may also be empty; an empty element
contributes no bytes of its own but still participates in delimiter placement, so
`join(["left", "", "right"], "-")` is `"left--right"`. The boundary cases follow
directly: the empty list yields `""`, and a single-element list yields that
element with no delimiter at all.

`join` and `strings::split` are inverses for a non-empty delimiter: joining the
result of `split(value, delimiter)` with the same delimiter reproduces `value`.

Neither argument is mutated; the result is a new owned `String`. `parts` is also
accepted under the name `values`, and `delimiter` under the name `separator`.
[[src/builtins/strings.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `parts` | `List OF String` | The strings to concatenate, in order. Any list is accepted, including the empty list and lists containing empty strings. Also accepted under the name `values`. [[src/builtins/strings.rs:call_param_names]] |
| `delimiter` | `String` | The separator placed between successive elements. Any `String`, including the empty one. Also accepted under the name `separator`. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding the elements of `parts` in order, separated by `delimiter`. The empty list yields `""`; a single-element list yields that element unchanged. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Join words with a separator, and with none:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::join(["Hello", "World"], " "))
  io::print(strings::join(["a", "b", "c"], ""))
  RETURN 0
END FUNC
```

Empty elements still take a delimiter, and the empty list yields the empty
string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET none AS List OF String = []
  io::print(strings::join(["left", "", "right"], "-"))
  io::print("[" & strings::join(none, ",") & "]")
  RETURN 0
END FUNC
```

Pass the arguments by name:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::join(parts := ["left", "right"], delimiter := "/"))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings split`
- `mfb man strings repeat`
- `mfb man strings padLeft`
- `mfb man collections append`
- `mfb man strings`
