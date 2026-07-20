# trim

Remove leading and trailing Unicode whitespace from a string.

## Synopsis

```
strings::trim(value AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::trim` returns a new `String` equal to `value` with every leading and
trailing whitespace scalar removed. Both ends are trimmed in one call; to trim
only one end use `strings::trimStart` or `strings::trimEnd`.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_trim]]

Whitespace is recognized by Unicode scalar, not by byte, and the recognized set
is exactly the Unicode `White_Space` property: `U+0009`–`U+000D` (tab, line
feed, vertical tab, form feed, carriage return), `U+0020` space, `U+0085` next
line, `U+00A0` no-break space, `U+1680` ogham space mark, `U+2000`–`U+200A` the
en/em quad and space family, `U+2028` line separator, `U+2029` paragraph
separator, `U+202F` narrow no-break space, `U+205F` medium mathematical space,
and `U+3000` ideographic space. Multi-byte whitespace scalars are matched whole,
so trimming never splits a scalar. [[src/target/shared/code/private/unicode.rs:emit_unicode_whitespace_branch]]

Only the contiguous runs of whitespace at the very start and the very end are
removed. Whitespace between non-whitespace scalars is interior and is preserved
byte for byte. A `value` that is entirely whitespace trims to the empty string,
and the empty string trims to the empty string. `value` is not mutated; the
result is a newly allocated `String`, even when nothing was trimmed.
[[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]]

The trim is locale-independent and performs no normalization or case folding. To
strip a specific set of scalars instead of whitespace, use `strings::trimChars`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to trim. Any `String` is accepted, including the empty string and a string that is entirely whitespace. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` equal to `value` with leading and trailing Unicode whitespace removed. The empty string, and any all-whitespace string, yield `""`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Remove surrounding spaces:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::trim("  Hello  "))
  RETURN 0
END FUNC
```

Interior whitespace is preserved, and non-ASCII whitespace is trimmed:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::trim("\n  a b  \n"))
  io::print(strings::trim("　wide　"))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings trimStart`
- `mfb man strings trimEnd`
- `mfb man strings trimChars`
- `mfb man strings stripPrefix`
- `mfb man strings`
