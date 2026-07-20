# stripSuffix

Remove one trailing occurrence of a suffix from a string.

## Synopsis

```
strings::stripSuffix(value AS String, suffix AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::stripSuffix` returns `value` with one trailing occurrence of `suffix`
removed when `value` ends with `suffix`, and returns `value` unchanged otherwise.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_strip]]

The match is an exact byte comparison of the trailing bytes of `value` against
every byte of `suffix`, with no normalization and no case folding. Because both
operands are well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte
suffix is always a whole-scalar suffix, so the remainder is always a valid
string. [[src/target/shared/code/private/unicode.rs:emit_string_byte_range_equal_branch]]

Exactly one copy is removed. If `value` ends with `suffix` repeated, only the
last copy is stripped and the earlier ones remain — call `stripSuffix` in a loop
to remove them all. An empty `suffix` removes no bytes, a `suffix` longer than
`value` cannot match, and a non-matching `suffix` leaves `value` alone; all three
return an equal string.

The function is total and never fails. Neither operand is modified, and a new
`String` is always allocated for the result, even on the unchanged path.
[[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]]

To test for the suffix without removing it, use `strings::endsWith`. To remove a
*set* of trailing scalars rather than a fixed substring, use
`strings::trimChars`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to strip from. May be empty. Returned as an equal copy when it does not end with `suffix`. [[src/builtins/strings.rs:call_param_names]] |
| `suffix` | `String` | The trailing substring to remove. May be empty, in which case `value` is returned unchanged. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | `value` with one trailing copy of `suffix` removed when it ends with `suffix`; otherwise a string equal to `value`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Remove a file extension; a non-matching suffix changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripSuffix("photo.png", ".png"))
  io::print(strings::stripSuffix("photo.png", ".jpg"))
  RETURN 0
END FUNC
```

Only one copy is removed:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripSuffix("foobarbar", "bar"))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings stripPrefix`
- `mfb man strings endsWith`
- `mfb man strings trimEnd`
- `mfb man strings trimChars`
- `mfb man strings`
