# trimEnd

Remove trailing Unicode whitespace from a string.

## Synopsis

```
strings::trimEnd(value AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::trimEnd` returns a new `String` equal to `value` with every trailing
whitespace scalar removed. Leading whitespace is left in place; it is the
one-sided form of `strings::trim`, which trims both ends.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_trim]]

Whitespace is recognized by Unicode scalar, not by byte, and the recognized set
is exactly the Unicode `White_Space` property: `U+0009`–`U+000D`, `U+0020`,
`U+0085`, `U+00A0`, `U+1680`, `U+2000`–`U+200A`, `U+2028`, `U+2029`, `U+202F`,
`U+205F`, and `U+3000`. Multi-byte whitespace scalars are matched whole, so
trimming never splits a scalar. [[src/target/shared/code/private/unicode.rs:emit_unicode_whitespace_branch]]

Removal stops at the last scalar that is not whitespace, so leading and interior
content, including embedded spaces and line breaks, is preserved byte for byte. A
`value` that is entirely whitespace yields the empty string, and the empty string
yields the empty string. `value` is not mutated; the result is a newly allocated
`String`, even when nothing was trimmed.
[[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to trim at the end. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` equal to `value` with trailing Unicode whitespace removed. The empty string, and any all-whitespace string, yield `""`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Remove trailing spaces while keeping the leading ones:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::trimEnd("  Hello  ") & "]")
  RETURN 0
END FUNC
```

Strip a trailing newline from a read line:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::trimEnd("value\n") & "]")
  RETURN 0
END FUNC
```

## See also

- `mfb man strings trim`
- `mfb man strings trimStart`
- `mfb man strings trimChars`
- `mfb man strings stripSuffix`
- `mfb man strings`
