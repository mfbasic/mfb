# stripPrefix

Remove one leading occurrence of a prefix from a string.

## Synopsis

```
strings::stripPrefix(value AS String, prefix AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::stripPrefix` returns `value` with one leading occurrence of `prefix`
removed when `value` begins with `prefix`, and returns `value` unchanged
otherwise. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_strip]]

The match is an exact byte comparison of the leading bytes of `value` against
every byte of `prefix`, with no normalization and no case folding. Because both
operands are well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte
prefix is always a whole-scalar prefix, so the remainder is always a valid
string. [[src/target/shared/code/private/unicode.rs:emit_string_byte_range_equal_branch]]

Exactly one copy is removed. If `value` begins with `prefix` repeated, only the
first copy is stripped and the rest remain — call `stripPrefix` in a loop to
remove them all. An empty `prefix` removes no bytes, a `prefix` longer than
`value` cannot match, and a non-matching `prefix` leaves `value` alone; all three
return an equal string.

The function is total and never fails. Neither operand is modified, and a new
`String` is always allocated for the result, even on the unchanged path.
[[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]]

To test for the prefix without removing it, use `strings::startsWith`. To remove
a *set* of leading scalars rather than a fixed substring, use
`strings::trimChars`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to strip from. May be empty. Returned as an equal copy when it does not begin with `prefix`. [[src/builtins/strings.rs:call_param_names]] |
| `prefix` | `String` | The leading substring to remove. May be empty, in which case `value` is returned unchanged. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | `value` with one leading copy of `prefix` removed when it begins with `prefix`; otherwise a string equal to `value`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Remove a leading scheme; a non-matching prefix changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripPrefix("https://example.com", "https://"))
  io::print(strings::stripPrefix("example.com", "https://"))
  RETURN 0
END FUNC
```

Only one copy is removed:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripPrefix("foofoobar", "foo"))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings stripSuffix`
- `mfb man strings startsWith`
- `mfb man strings trimStart`
- `mfb man strings trimChars`
- `mfb man strings`
