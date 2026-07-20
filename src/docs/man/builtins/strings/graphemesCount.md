# graphemesCount

Count the extended grapheme clusters in a string.

## Synopsis

```
strings::graphemesCount(value AS String) AS Integer
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::graphemesCount` returns the number of Unicode extended grapheme
clusters in `value`. It is defined as the element count of
`strings::graphemes(value)`, and is computed by performing that same
segmentation and reading the resulting list's length.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_graphemes_count]]

An extended grapheme cluster is one user-perceived character, and it may be built
from several Unicode scalar values: a base letter followed by combining marks, a
flag formed from a pair of regional indicators, or an emoji built from a base
symbol joined to modifiers by zero-width joiners. Each such cluster counts as
one. [[src/target/shared/code/private/unicode.rs:emit_grapheme_break_branch]]

The count is therefore a third measure, distinct from both `len(value)`, which
counts Unicode scalar values, and `strings::byteLen(value)`, which counts UTF-8
bytes. For text with combining marks, emoji, or characters outside the Basic
Multilingual Plane, all three can differ: `"e"` plus `U+0301` plus `"fg"` has
three clusters but four scalars.

The empty string yields `0`. `value` is not mutated and the call never fails.
Because the count is derived by segmenting the whole string, it is a linear scan,
not a stored field — prefer calling `strings::graphemes` once when you need both
the clusters and their count.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose clusters are counted. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of extended grapheme clusters in `value`, a non-negative `Integer`. `0` for the empty string. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Count user-perceived characters:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::graphemesCount("abc")))
  io::print(toString(strings::graphemesCount("a😀b")))
  RETURN 0
END FUNC
```

A combining sequence counts as one cluster but two scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET text AS String = "e" & "́" & "fg"
  io::print(toString(strings::graphemesCount(text)))
  io::print(toString(len(text)))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings graphemes`
- `mfb man strings graphemeAt`
- `mfb man strings byteLen`
- `mfb man general len`
- `mfb man unicode`
- `mfb man strings`
