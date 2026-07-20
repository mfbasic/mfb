# replace

Return a list with every element equal to a given value replaced

## Synopsis

```
collections::replace OF T(value AS List OF T, old AS T, new AS T) AS List OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

## Description

`collections::replace` returns a new list of the same length as `value` in which
every element equal to `old` has been replaced by `new`, and every other element
is carried over unchanged. It takes exactly three arguments; none is optional and
none is variadic. [[src/builtins/collections.rs:arity]]

All matches are replaced, not just the first, and positions are preserved: the
result has the same length and the same ordering as `value`, differing only at
the indices where `old` occurred. When `old` does not occur, the result is a copy
of `value`. When `value` is empty, the result is empty.
[[src/target/shared/code/builder_strings.rs:lower_list_replace]]

Matching compares each element's stored payload against `old` using the same
element-equality test the rest of the collections layer uses, so the element type
must be one for which that comparison is defined; `old` and `new` must both have
exactly the element type `T`. `new` may itself be equal to `old`, in which case
the result is equal to `value`.
[[src/target/shared/code/builder_strings.rs:lower_list_replace]]

Only the **List** overload of `replace` lives in `collections`. The `String`
overload — replacing a substring within a `String` — is a different function that
lives in `strings::`. A `String` first argument does not resolve here.
[[src/builtins/general.rs:resolve_replace_list]]

`replace` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. There is no in-place fast path for `replace` —
the compiler's in-place assignment recognizers cover `append`, bulk `append`,
`prepend`, `set`, and string concatenation, not `replace`.
[[src/target/shared/code/builder_inplace_assign.rs:try_inplace_set_assign]]

`replace` is **infallible**: no path in its lowering raises a trappable domain
error. It has no index to range-check, and a `new` that never matches is a
success producing an unchanged copy, not a failure — so it is classified as
infallible alongside `append` and `prepend`, and an inline `TRAP` written on a
`replace` call has a dead handler (the front end reports
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Allocation exhaustion is not a trappable domain
error in this language. [[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to scan; left unchanged. Also accepted under the name `list`. Must be a list type; a `String` argument selects `strings::replace` instead and does not resolve here. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_replace_list]] |
| `old` | `T` | The element value to look for. Every element equal to it is replaced. Also accepted under the name `needle`. Its type must be exactly the element type `T`. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_replace_list]] |
| `new` | `T` | The element value written in place of each match. Also accepted under the name `replacement`. Its type must be exactly the element type `T`. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_replace_list]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the same type and the same length as `value`, with every occurrence of `old` replaced by `new`. Equal to `value` when `old` does not occur, and empty when `value` is empty. [[src/builtins/general.rs:resolve_replace_list]] |

## Errors

No errors.

## Type checking

The first argument must be a `List OF T`, and both `old` and `new` must have
exactly that list's element type `T`. There is no implicit widening or
conversion, so `replace` on a `List OF Float` does not accept an `Integer`
needle. Any other combination — a non-list first argument, a mismatched `old` or
`new`, or a wrong argument count — resolves to no overload and is rejected at
compile time. [[src/builtins/general.rs:resolve_replace_list]]

## Examples

Replace every matching element:

```
IMPORT collections

FUNC main AS Integer
  LET values AS List OF Integer = collections::replace([1, 2, 1], 1, 9)
  RETURN 0
END FUNC
```

A needle that does not occur yields an unchanged copy:

```
IMPORT collections
IMPORT strings
IMPORT io

FUNC main AS Integer
  LET words AS List OF String = collections::replace(["a", "b"], "z", "Q")
  io::print(strings::join(words, ","))
  RETURN 0
END FUNC
```

Substituting a placeholder throughout a list:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET cleaned AS List OF String = collections::replace(["x", "b", "x"], "x", "QQ")
  io::print(toString(len(cleaned)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections set`
- `mfb man collections find`
- `mfb man collections contains`
- `mfb man collections transform`
- `mfb man strings replace`
- `mfb man collections`
