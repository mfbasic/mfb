# replace

Replace every non-overlapping occurrence of a substring.

## Synopsis

```
strings::replace(value AS String, old AS String, new AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::replace` returns a new `String` in which every non-overlapping
occurrence of `old` in `value` has been replaced with `new`.
[[src/target/shared/code/builder_strings.rs:lower_replace]]

Scanning runs left to right. At each match the replacement is emitted and
scanning resumes immediately after the matched region, so matches never overlap
and a replacement is never re-examined: replacing `"aba"` with `"x"` in
`"ababa"` gives `"xba"`, not `"xx"` or `"xa"`. Where `old` does not match, the
original bytes are copied through unchanged.

Matching is an exact byte comparison. `replace` performs no Unicode
normalization, no case folding, and no grapheme-cluster awareness — `old` must
match byte for byte. Because both operands are well-formed UTF-8 and UTF-8 is
self-synchronizing, a byte match is always a whole-scalar match, so the result is
always well-formed UTF-8.

If `old` is the empty string, nothing can match and a copy of `value` is
returned; `replace` never inserts `new` between existing scalars. If `old` is
longer than `value` it likewise cannot match. When `old` does match and `new` is
empty, each match is deleted.

None of the three arguments is mutated. The result is always a freshly allocated
`String` — when no replacement occurred, `value` is deep-copied rather than
aliased, so the caller owns the returned value unconditionally.

`old` is also accepted under the name `needle`, and `new` under the name
`replacement`. The bare `replace` name is also defined for lists; see
`mfb man collections replace`. [[src/builtins/strings.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to copy from, replacing matches as they are found. [[src/builtins/strings.rs:call_param_names]] |
| `old` | `String` | The substring to search for. Also accepted under the name `needle`. An empty `old`, or one longer than `value`, never matches. [[src/builtins/strings.rs:call_param_names]] |
| `new` | `String` | The text written in place of each match. Also accepted under the name `replacement`. May be empty, which deletes each match. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` with every non-overlapping occurrence of `old` replaced by `new`. Equal to `value` when there are no matches. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Replace every occurrence, and delete with an empty replacement:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::replace("hello", "l", "x"))
  io::print(strings::replace("banana", "na", ""))
  RETURN 0
END FUNC
```

Matches never overlap, and an empty `old` changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::replace("ababa", "aba", "x"))
  io::print(strings::replace("hi", "", "x"))
  RETURN 0
END FUNC
```

Pass the arguments by name:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::replace(value := "hello", old := "l", new := "q"))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings find`
- `mfb man strings contains`
- `mfb man strings split`
- `mfb man strings mid`
- `mfb man collections replace`
- `mfb man strings`
