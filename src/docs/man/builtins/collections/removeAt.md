# removeAt

Return a list with the element at a given index removed

## Synopsis

```
collections::removeAt OF T(value AS List OF T, index AS Integer) AS List OF T
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

`collections::removeAt` returns a new list containing every element of `value`
except the one at `index`, with the elements above `index` shifted down by one to
close the gap and all other relative order preserved. The result is always
exactly one element shorter than `value`. It takes exactly two arguments; neither
is optional and neither is variadic. [[src/builtins/collections.rs:arity]]

`index` is zero-based and is validated as `0 <= index < len(value)`. The upper
bound is **exclusive**: unlike `collections::insert`, `index` equal to the length
is not a valid position — there is nothing there to remove — and raises
`ErrIndexOutOfRange`, as does any negative `index`. Removing from an empty list
therefore always raises, since no index satisfies the range.
[[src/target/shared/code/list_mutate.rs:lower_list_remove_at]]

`removeAt` is value-semantic. The list named by `value` is unchanged; the
shortened list is the returned value, and a program observes the update only
through what it does with that return value. There is no in-place fast path for
`removeAt` — the compiler's in-place assignment recognizers cover `append`, bulk
`append`, `prepend`, `set`, and string concatenation, not `removeAt`.
[[src/target/shared/code/builder_inplace_assign.rs:try_inplace_set_assign]]

`removeAt` is **fallible**: the range check is a real trappable domain error, so
an inline `TRAP` on a `removeAt` call compiles and catches the out-of-range
failure rather than being reported as a dead handler. The bounds test runs before
the result block is allocated, so a rejected index allocates nothing.
[[src/builtins/mod.rs:inline_builtin_raw_supported]]
[[src/target/shared/code/list_mutate.rs:lower_list_remove_at]]

`removeAt` operates on lists only. To drop a key from a `Map OF K TO V`, use
`collections::removeKey`, which takes a key rather than an index and does not
raise when the key is absent. [[src/builtins/general.rs:resolve_remove_key]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to remove from; left unchanged. Also accepted under the name `list`. Must be a list type; a `Map` or scalar resolves no overload and is a compile-time error. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_remove_at]] |
| `index` | `Integer` | Zero-based position of the element to remove. Valid range is `0` through `len(value) - 1` inclusive. Must be declared `Integer` exactly — no other numeric type resolves. This parameter has no alternate spelling. [[src/builtins/collections.rs:call_param_names]] [[src/target/shared/code/list_mutate.rs:lower_list_remove_at]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the same type as `value`, one element shorter, without the element that was at `index`. Removing index `0` drops the first element; removing `len(value) - 1` drops the last. [[src/builtins/general.rs:resolve_remove_at]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `index` is negative, or `index` is greater than or equal to `len(value)`. This includes every call on an empty list. [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] [[src/target/shared/code/list_mutate.rs:lower_list_remove_at]] |

## Type checking

The first argument must be a `List OF T` and the second must be `Integer`. There
is no implicit widening or conversion. The result has the same list type as
`value`. A call on a non-list first argument or a non-`Integer` index resolves to
no overload and is rejected at compile time; the index range itself is a runtime
check, not a compile-time one. [[src/builtins/general.rs:resolve_remove_at]]

## Examples

Remove the second element:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::removeAt([1, 2, 3], 1)
  RETURN 0
END FUNC
```

Remove the last element:

```
IMPORT collections

FUNC main AS Integer
  LET source AS List OF String = ["a", "b", "c"]
  LET shorter AS List OF String = collections::removeAt(source, len(source) - 1)
  RETURN 0
END FUNC
```

Catch an out-of-range index with an inline `TRAP`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2]
  LET shorter AS List OF Integer = collections::removeAt(numbers, 2) TRAP(e)
    io::print(e.message)
    RECOVER numbers
  END TRAP
  RETURN 0
END FUNC
```

## See also

- `mfb man collections insert`
- `mfb man collections removeKey`
- `mfb man collections set`
- `mfb man collections append`
- `mfb man collections`
