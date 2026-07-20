# sort

Return a new list holding the elements of a list in ascending order

## Synopsis

```
collections::sort OF T(value AS List OF T) AS List OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:FUNCTIONS]]

## Description

`collections::sort` returns a new list containing every element of `value`
arranged in ascending order. It is a generic function written in MFBASIC source:
a call to `collections::sort` is rewritten to the internal
`__collections_sort` generic and instantiated for the element type `T` during
monomorphization. [[src/builtins/collections.rs:internal_name]]
[[src/builtins/collections_package.mfb:__collections_sort]]

The algorithm is a bottom-up merge sort with O(n log n) comparisons. Runs of
width 1 are merged into runs of width 2, then 4, and so on, until a single run
covers the list. The merge is **stable**: when a left-run element and a
right-run element compare equal, the left-run element is emitted first, because
the right-run element is taken only when it is *strictly less than* the left-run
element. Elements that compare equal therefore keep their original relative
order. [[src/builtins/collections_package.mfb:__collections_sort]]

Ordering is determined entirely by the `<` operator applied to whole elements.
For the numeric element types that is numeric order; for `String` it is the
ordering the `<` operator defines on strings; for `Scalar` it is codepoint
order. There is no descending form, no comparison-function parameter, and no
locale or case-insensitivity option. To sort by something other than the element
itself, use `collections::sortBy`, which orders by a computed key.

When `value` has fewer than two elements — that is, when it is empty or holds a
single element — `sort` returns `value` unchanged without performing any
comparison. [[src/builtins/collections_package.mfb:__collections_sort]]

`value` is not modified. Like every `collections` helper, `sort` produces a new
list value and leaves its argument intact.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to order. Any length is accepted, including the empty list. `T` must be a type the `<` operator accepts. Named-argument spelling is `value`. [[src/builtins/collections_package.mfb:__collections_sort]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list with the same elements as `value` in ascending order, equal elements in their original relative order. For a list of fewer than two elements, `value` itself. [[src/builtins/collections_package.mfb:__collections_sort]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The arena cannot allocate a list the merge passes need to build. Ordering itself never fails: no comparison, index, or length is rejected. [[src/target/shared/code/builder_collection_mutate.rs:lower_list_insert_collection]] [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Type checking

`T` is inferred from the argument. `sort` compares whole elements with `<`, so
the instantiated element type must be one the `<` operator accepts: `Integer`,
`Byte`, `Float`, `Fixed`, `Money`, `String`, or `Scalar`. `Money` compares only
against `Money`, and `Scalar` never orders against `String`.
[[src/ir/verify/mod.rs:check_binary_operands]]

The constraint is enforced after monomorphization, when the generic body has
been instantiated for a concrete `T`. Sorting a list whose element type the `<`
operator does not accept — a `Boolean`, a record, a nested `List`, or a `Map` —
is a compile-time `TYPE_BINARY_OPERATOR_MISMATCH` error reported against the
comparison inside the merge, not a runtime failure.
[[src/ir/verify/mod.rs:check_binary_operands]]

## Examples

Sort a list of integers:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ordered AS List OF Integer = collections::sort([3, 1, 2])
  io::print(toString(collections::get(ordered, 0)))
  RETURN 0
END FUNC
```

Sort strings, and observe that the input is untouched:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET names AS List OF String = ["pear", "apple", "fig"]
  LET ordered AS List OF String = collections::sort(names)
  io::print(collections::get(ordered, 0))
  io::print(collections::get(names, 0))
  RETURN 0
END FUNC
```

A list of fewer than two elements is returned as-is:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET one AS List OF Integer = collections::sort([7])
  io::print(toString(len(one)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections sortBy`
- `mfb man collections distinct`
- `mfb man collections chunks`
- `mfb man collections`
