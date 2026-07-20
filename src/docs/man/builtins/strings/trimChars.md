# trimChars

Remove leading and trailing scalars that belong to a given set.

## Synopsis

```
strings::trimChars(value AS String, chars AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::trimChars` returns a new `String` equal to `value` with every leading
and trailing Unicode scalar that appears in `chars` removed. `chars` is treated
as an unordered *set* of scalars, not as a sequence: order, position, and
repetition within `chars` are irrelevant, and `chars` is never matched as a
substring. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_trim_chars]]

Trimming works from both ends toward the middle. From the front, whole scalars
are consumed while each one is a member of the set, stopping at the first scalar
that is not; the same is then done from the back. Only those two contiguous runs
are removed, so a set member that sits between two non-members is interior and is
preserved. Membership is tested on whole Unicode scalars rather than bytes, so a
multi-byte scalar listed in `chars` matches correctly and trimming never splits a
scalar. [[src/target/shared/code/builder_strings_package.rs:emit_chars_set_contains_branch]]

Comparison is literal: scalars must be equal exactly, with no normalization and
no case folding, so a scalar removed in one case is not removed in another. When
`chars` is the empty string the set is empty, nothing qualifies, and a copy of
`value` is returned. When `value` is empty, or every scalar of `value` belongs to
the set, the result is the empty string. Neither argument is mutated; the result
is a newly allocated `String`, even when nothing was trimmed.
[[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]]

Unlike `strings::trim`, which removes Unicode whitespace, `trimChars` removes
only what `chars` lists. To strip a fixed leading or trailing *substring* rather
than a set of scalars, use `strings::stripPrefix` or `strings::stripSuffix`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to trim. May be empty. Returned as an equal copy when it has no leading or trailing member of `chars`. [[src/builtins/strings.rs:call_param_names]] |
| `chars` | `String` | The set of Unicode scalars to remove from both ends. Interpreted as a set; order and repetition do not matter. May be empty, in which case `value` is returned unchanged. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` equal to `value` with all leading and trailing members of `chars` removed. Returns `""` when `value` is empty or consists entirely of set members. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Trim a set of surrounding scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::trimChars("xxhelloxx", "x"))
  io::print(strings::trimChars("xxyHelloxyx", "xy"))
  RETURN 0
END FUNC
```

Interior set members are preserved, and an empty `chars` changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::trimChars("--a-b--", "-"))
  io::print(strings::trimChars("hello", ""))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings trim`
- `mfb man strings trimStart`
- `mfb man strings trimEnd`
- `mfb man strings stripPrefix`
- `mfb man strings stripSuffix`
- `mfb man strings`
