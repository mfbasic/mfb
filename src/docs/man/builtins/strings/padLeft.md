# padLeft

Pad a string on the left to a given scalar width.

## Synopsis

```
strings::padLeft(value AS String, width AS Integer) AS String
strings::padLeft(value AS String, width AS Integer, padChar AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::padLeft` returns a new `String` in which copies of `padChar` are
prepended to `value` until the whole result spans `width` Unicode scalar values.
The number of copies prepended is `width` minus the current scalar length of
`value`. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_pad]]

Width is counted in Unicode scalar values, not in UTF-8 bytes and not in grapheme
clusters, and it counts scalars of the *result*, not of the padding alone. A
multi-byte `padChar` therefore contributes one toward the width per copy while
adding several bytes: `padLeft("x", 3, "😀")` is `"😀😀x"`.

When the scalar length of `value` already equals or exceeds `width`, no padding
is added and the result equals `value`. `padLeft` never truncates to fit within
`width`. Note that a new `String` is always allocated, even in that case; the
original is never aliased.

`padChar` is optional and defaults to a single space. When supplied, it must be
exactly one well-formed Unicode scalar value — neither empty nor more than one
scalar — otherwise `ErrInvalidArgument` is raised. A negative `width` raises the
same error, as does a result size that cannot be represented in 64 bits.
[[src/target/shared/code/builder_error_emission.rs:emit_checked_size_multiply]]

Neither argument is mutated.

## Overloads

**`strings::padLeft(value AS String, width AS Integer) AS String`**

Pads with a single space (`" "`), materialized internally so the two forms share
one code path. [[src/builtins/strings.rs:resolve_call]]

**`strings::padLeft(value AS String, width AS Integer, padChar AS String) AS String`**

Pads with the supplied `padChar`, which must be exactly one Unicode scalar value.
[[src/builtins/strings.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to pad. Returned as an equal copy when its scalar length is already at least `width`. [[src/builtins/strings.rs:call_param_names]] |
| `width` | `Integer` | The target total length of the result in Unicode scalar values. Must be `0` or greater; `0` never pads. [[src/builtins/strings.rs:call_param_names]] |
| `padChar` | `String` | Optional. The fill character prepended to reach `width`; defaults to a single space. Must be exactly one Unicode scalar value. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` equal to `value` prefixed with enough copies of `padChar` to span `width` scalars, or equal to `value` when it is already that long. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `width` is negative; `padChar` is empty, is more than one scalar, or is not a well-formed single UTF-8 scalar; or the result size cannot be represented in 64 bits. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_pad]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Left-pad with the default space and with an explicit character:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::padLeft("42", 5) & "]")
  io::print(strings::padLeft("42", 5, "0"))
  RETURN 0
END FUNC
```

An already-long value is never truncated, and a multi-byte pad counts as one
scalar per copy:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::padLeft("hello", 3))
  io::print(strings::padLeft("x", 3, "😀"))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings padRight`
- `mfb man strings left`
- `mfb man strings right`
- `mfb man strings repeat`
- `mfb man strings`
