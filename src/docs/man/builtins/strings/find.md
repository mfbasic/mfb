# find

Locate the first occurrence of a substring, by Unicode scalar index.

## Synopsis

```
strings::find(value AS String, needle AS String) AS Integer
strings::find(value AS String, needle AS String, start AS Integer) AS Integer
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::find` searches `value` for the first occurrence of `needle` at or after
the scalar position `start`, and returns the zero-based scalar index where that
occurrence begins. [[src/target/shared/code/builder_search.rs:lower_find]]

Positions are measured in Unicode scalar values — not UTF-8 bytes and not
grapheme clusters. A multi-byte scalar such as `é` or `😀` counts as one
position even though it occupies several bytes. Both `start` and the returned
index are scalar indexes, so `find("a😀é", "😀")` is `1`. Matching itself is an
exact byte comparison with no normalization and no case folding, so a
precomposed `é` does not match a decomposed one.
[[src/target/shared/code/private/unicode.rs:emit_utf8_decode_next]]

`start` defaults to `0` when the two-argument form is used. It must lie in `0`
through the scalar length of `value` *inclusive*; the upper bound equals the
length so a search may begin at the very end of the string, where only an empty
needle can match. A negative `start`, or one past the scalar length, raises
`ErrIndexOutOfRange`. An empty `needle` matches immediately and returns `start`.
[[src/builtins/strings.rs:arity]]

`find` always returns a valid index on success and never reports absence with a
sentinel such as `-1`. When `needle` does not occur at or after `start` it raises
`ErrNotFound`. When absence is an ordinary, expected outcome, guard the call with
`strings::contains` and call `find` only once a match is known to exist.

`find` does not mutate either operand. The bare `find` name is also defined for
lists; see `mfb man collections find` for the `List` form.

## Overloads

**`strings::find(value AS String, needle AS String) AS Integer`**

Searches the whole of `value` from the beginning; equivalent to passing a `start`
of `0`. [[src/builtins/strings.rs:resolve_call]]

**`strings::find(value AS String, needle AS String, start AS Integer) AS Integer`**

Begins the search at scalar index `start`, ignoring any earlier occurrence. Used
to walk successive matches. [[src/builtins/strings.rs:resolve_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to search. May be empty. [[src/builtins/strings.rs:call_param_names]] |
| `needle` | `String` | The substring to locate. An empty `needle` matches at `start`. [[src/builtins/strings.rs:call_param_names]] |
| `start` | `Integer` | Optional. The zero-based scalar index at which to begin searching. Defaults to `0`. Must be in `0` through the scalar length of `value` inclusive. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The zero-based Unicode scalar index at which the first occurrence of `needle` at or after `start` begins. Returns `start` when `needle` is empty. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `start` is negative, or greater than the scalar length of `value`. [[src/target/shared/code/builder_search.rs:lower_find]] [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |
| `77050004` | `ErrNotFound` | No occurrence of `needle` exists at or after `start`. [[src/target/shared/code/builder_search.rs:lower_find]] [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |

## Examples

Find the first occurrence, then resume after it:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::find("hello", "l")))
  io::print(toString(strings::find("hello", "l", 3)))
  RETURN 0
END FUNC
```

Indexes are scalar positions, not byte offsets:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::find("a😀é", "😀")))
  io::print(toString(strings::find("aé日é", "日")))
  RETURN 0
END FUNC
```

Guard with `contains`, or catch the absence with `TRAP`:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET at AS Integer = strings::find("hello", "z") TRAP(e)
    io::print("absent")
    RETURN 0
  END TRAP
  io::print(toString(at))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings contains`
- `mfb man strings mid`
- `mfb man strings count`
- `mfb man strings replace`
- `mfb man collections find`
- `mfb man strings`
