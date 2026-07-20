# mid

Return a new list holding a contiguous run of elements taken from a list

## Synopsis

```
collections::mid OF T(value AS List OF T, start AS Integer, count AS Integer) AS List OF T
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

`collections::mid` returns a new list holding the `count` elements of `value`
that begin at the zero-based index `start`, in their original order. It is a
**native** member: the compiler emits the slice loop directly rather than
instantiating an MFBASIC generic. [[src/builtins/collections.rs:is_native_member]]
[[src/target/shared/code/builder_search.rs:lower_list_mid]]

This page documents the `List` form only. `collections::mid` accepts nothing but
a `List` as its first argument; the `String` slice of the same name lives in
`strings::`. [[src/builtins/general.rs:resolve_mid_list]]

All three arguments are required — there is no two-argument "to the end" form —
and `start` and `count` must both be exactly `Integer`.
[[src/builtins/collections.rs:arity]] [[src/builtins/general.rs:resolve_mid_list]]

The range is **validated, not clamped**. Before any element is copied the
lowering checks, in order, that `start` is not negative, that `count` is not
negative, that `start` is not greater than the length of `value`, that
`start + count` does not wrap around, and that `start + count` is not greater
than the length of `value`. Any of those failing raises `ErrIndexOutOfRange`.
A short trailing run is therefore an error rather than a truncated result: on a
three-element list, `mid(value, 2, 2)` fails instead of returning one element.
[[src/target/shared/code/builder_search.rs:lower_list_mid]]

Empty results are legal at the boundaries, since `start` may equal the length of
`value` and `count` may be `0`: on a four-element list, `mid(value, 4, 0)`
returns an empty list.
[[src/target/shared/code/builder_search.rs:lower_list_mid]]

The result is a freshly allocated, independently owned list of the same type as
`value`; `value` itself is neither modified nor consumed, and element payloads
are copied into the new list's own data region rather than shared.
[[src/builtins/general.rs:resolve_mid_list]]
[[src/target/shared/code/builder_search.rs:lower_list_mid]]

`mid` copies the selected run using a fast contiguous path when the source
entries covering the slice are stored in order and packed tightly, and falls
back to a per-entry copy otherwise. A list whose entry records have been
permuted without moving the underlying data — the result of a sorted directory
listing, for instance — takes the fallback. Either way the returned elements are
the same. [[src/target/shared/code/builder_search.rs:lower_list_mid]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to slice. Must be a `List`; a `String` first argument selects `strings::mid` instead. Also accepted under the name `list`. [[src/builtins/general.rs:resolve_mid_list]] [[src/builtins/collections.rs:call_param_names]] |
| `start` | `Integer` | Zero-based index of the first element to take. Must be `0` or greater and no greater than the length of `value`. Required — it has no default. There is no alternate name for this parameter. [[src/target/shared/code/builder_search.rs:lower_list_mid]] [[src/builtins/collections.rs:call_param_names]] |
| `count` | `Integer` | How many elements to take. Must be `0` or greater, and `start + count` must not exceed the length of `value`. Required. There is no alternate name for this parameter. [[src/target/shared/code/builder_search.rs:lower_list_mid]] [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the same type as `value` holding exactly `count` elements starting at index `start`, in their original order. An empty list when `count` is `0`. [[src/builtins/general.rs:resolve_mid_list]] [[src/target/shared/code/builder_search.rs:lower_list_mid]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `start` is negative, `count` is negative, `start` is greater than the length of `value`, `start + count` overflows, or `start + count` is greater than the length of `value`. [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] [[src/target/shared/code/builder_search.rs:lower_list_mid]] |

## Type checking

`T` is the element type of `value`, which must be a `List`. `start` and `count`
must both be exactly `Integer`; no other numeric type is accepted and no
conversion is applied. The call takes exactly three arguments, and the result
type is the same `List OF T` as the input.
[[src/builtins/general.rs:resolve_mid_list]] [[src/builtins/collections.rs:arity]]

## Examples

Take two elements from the middle:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3, 4]
  LET middle AS List OF Integer = collections::mid(numbers, 1, 2)
  io::print(toString(collections::get(middle, 0)))
  io::print(toString(len(middle)))
  RETURN 0
END FUNC
```

An empty slice at the end of the list is legal:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3, 4]
  LET empty AS List OF Integer = collections::mid(numbers, 4, 0)
  io::print(toString(len(empty)))
  RETURN 0
END FUNC
```

An over-long range raises rather than truncating, so handle it:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  LET tail AS List OF Integer = collections::mid(numbers, 2, 2) TRAP(e)
    io::print("bad range: " & e.message)
    RECOVER []
  END TRAP
  io::print(toString(len(tail)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections find`
- `mfb man collections take`
- `mfb man collections drop`
- `mfb man collections get`
- `mfb man strings mid`
- `mfb man collections`
