# graphemeAt

Return the extended grapheme cluster at a grapheme index.

## Synopsis

```
strings::graphemeAt(value AS String, index AS Integer) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::graphemeAt` returns the single extended grapheme cluster at the
zero-based grapheme `index` of `value`, as a new `String`.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_grapheme_at]]

Indexing is by cluster, not by Unicode scalar value (as `len` counts) and not by
UTF-8 byte (as `strings::byteLen` counts). It uses the same enumeration
`strings::graphemes` produces, so `graphemeAt(value, i)` is the element at
position `i` of `strings::graphemes(value)`. An extended grapheme cluster is one
user-perceived character and may be several scalars — a base letter plus
combining marks, a regional-indicator flag pair, or an emoji ZWJ sequence — so
the returned string may hold multiple scalars and many bytes.
[[src/target/shared/code/private/unicode.rs:emit_grapheme_break_branch]]

`index` is zero-based: `0` selects the first cluster and the last valid index is
`strings::graphemesCount(value) - 1`. An index that is negative, or at or beyond
the cluster count, is out of range and raises `ErrIndexOutOfRange`; it is never
clamped or wrapped. The empty string has no clusters, so every index is out of
range for it.

`value` is not mutated; the returned `String` is a fresh copy of the selected
cluster.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to index. Any `String` is accepted, though every index is out of range for the empty string. [[src/builtins/strings.rs:call_param_names]] |
| `index` | `Integer` | The zero-based grapheme index to retrieve. Must satisfy `0 <= index < strings::graphemesCount(value)`. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding exactly the one extended grapheme cluster at `index`, which may span several scalars and several bytes. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `index` is negative, or is at or beyond the grapheme-cluster count of `value` — including every index applied to the empty string. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_grapheme_at]] [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |

## Examples

Retrieve a cluster by index:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::graphemeAt("abc", 1))
  io::print(strings::graphemeAt("a😀b", 1))
  RETURN 0
END FUNC
```

Guard the index against the cluster count:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET text AS String = "abc"
  IF strings::graphemesCount(text) > 5 THEN
    io::print(strings::graphemeAt(text, 5))
  ELSE
    io::print("too short")
  END IF
  RETURN 0
END FUNC
```

## See also

- `mfb man strings graphemes`
- `mfb man strings graphemesCount`
- `mfb man strings mid`
- `mfb man general len`
- `mfb man unicode`
- `mfb man strings`
