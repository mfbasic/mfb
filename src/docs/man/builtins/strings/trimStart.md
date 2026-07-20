# trimStart

Remove leading Unicode whitespace from a string.

## Synopsis

```
strings::trimStart(value AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::trimStart` returns a new `String` equal to `value` with every leading
whitespace scalar removed. Trailing whitespace is left in place; it is the
one-sided form of `strings::trim`, which trims both ends.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_trim]]

Whitespace is recognized by Unicode scalar, not by byte, and the recognized set
is exactly the Unicode `White_Space` property: `U+0009`–`U+000D`, `U+0020`,
`U+0085`, `U+00A0`, `U+1680`, `U+2000`–`U+200A`, `U+2028`, `U+2029`, `U+202F`,
`U+205F`, and `U+3000`. Multi-byte whitespace scalars are matched whole, so
trimming never splits a scalar. [[src/target/shared/code/private/unicode.rs:emit_unicode_whitespace_branch]]

Removal stops at the first scalar that is not whitespace, so interior and
trailing content, including embedded spaces and line breaks, is preserved byte
for byte. A `value` that is entirely whitespace yields the empty string, and the
empty string yields the empty string. `value` is not mutated; the result is a
newly allocated `String`, even when nothing was trimmed.
[[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to trim at the front. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` equal to `value` with leading Unicode whitespace removed. The empty string, and any all-whitespace string, yield `""`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Remove leading spaces while keeping the trailing ones:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::trimStart("  Hello  ") & "]")
  RETURN 0
END FUNC
```

Non-ASCII whitespace is trimmed too:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::trimStart("　wide　") & "]")
  RETURN 0
END FUNC
```

## See also

- `mfb man strings trim`
- `mfb man strings trimEnd`
- `mfb man strings trimChars`
- `mfb man strings stripPrefix`
- `mfb man strings`
