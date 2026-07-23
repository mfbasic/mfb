# sortBy

Return a new list ordered ascending by a key computed from each element

## Synopsis

```
collections::sortBy OF T, U(value AS List OF T, keyFn AS FUNC(T) AS U) AS List OF T
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

`collections::sortBy` returns a new list containing every element of `value`,
arranged in ascending order of the key `keyFn(element)`. The elements themselves
are never compared; only the keys are. It is a generic function written in
MFBASIC source, rewritten to the internal `__collections_sortBy` generic and
instantiated for the element type `T` and key type `U` during monomorphization.
[[src/builtins/collections.rs:internal_name]]
[[src/builtins/collections_package.mfb:__collections_sortBy]]

`keyFn` is applied to the whole list up front, in one pass, via
`collections::transform`, producing a parallel list of keys. Each element's key
is therefore computed **exactly once**, no matter how many comparisons that
element takes part in. `keyFn` must be a function value — for example a named
`FUNC` — and it is called once per element, in index order, before any
comparison happens. [[src/builtins/collections_package.mfb:__collections_sortBy]]

The sort is a bottom-up merge sort with O(n log n) comparisons, merging runs of
width 1, then 2, then 4, and so on. Items and their keys are carried through the
merge in parallel, so an element always travels with its own key. The merge is
**stable**: a right-run item is taken only when its key is *strictly less than*
the left-run item's key, so elements whose keys compare equal keep their original
relative order. [[src/builtins/collections_package.mfb:__collections_sortBy]]

When `value` has fewer than two elements, `sortBy` returns `value` unchanged.
In that case `keyFn` is never called, because the key pass is skipped along with
the sort. [[src/builtins/collections_package.mfb:__collections_sortBy]]

There is no descending form. To order descending by a numeric key, have `keyFn`
return the negated key. `value` is not modified.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to order. Any length is accepted, including the empty list. Named-argument spelling is `value`. [[src/builtins/collections_package.mfb:__collections_sortBy]] |
| `keyFn` | `FUNC(T) AS U` | The key projection, applied once to each element. Must be a function value taking one element and returning the sort key; `U` must be a type the `<` operator accepts. Named-argument spelling is `keyFn`. [[src/builtins/collections_package.mfb:__collections_sortBy]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list holding the elements of `value` ordered by ascending `keyFn(element)`, with equal-key elements in their original relative order. For a list of fewer than two elements, `value` itself. [[src/builtins/collections_package.mfb:__collections_sortBy]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The arena cannot allocate the key list or a list the merge passes need to build. Ordering itself never fails: no key, index, or length is rejected. [[src/target/shared/code/builder_collection_mutate.rs:lower_list_insert_collection]] [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

An error raised inside `keyFn` is not caught here; it propagates to the caller as
that function's own failure.

## Type checking

`T` is the element type and `U` is the return type of `keyFn`; both are inferred
from the arguments. The ordering constraint applies to `U`, **not** to `T`: the
keys are what `<` is applied to, so `U` must be one of the types the `<` operator
accepts — `Integer`, `Byte`, `Float`, `Fixed`, `Money`, `String`, or `Scalar`.
`Money` compares only against `Money`, and `Scalar` never orders against
`String`. [[src/ir/verify/values.rs:check_binary_operands]]

`T` carries no ordering requirement. A list of records, of nested lists, or of
any other unordered element type sorts fine as long as `keyFn` projects it to an
orderable `U`. A `U` the `<` operator does not accept is a compile-time
`TYPE_BINARY_OPERATOR_MISMATCH` error, reported after monomorphization against
the key comparison inside the merge. [[src/ir/verify/values.rs:check_binary_operands]]

## Examples

Sort descending by negating an integer key:

```
IMPORT collections
IMPORT io

FUNC negated(n AS Integer) AS Integer
  RETURN 0 - n
END FUNC

FUNC main AS Integer
  LET ordered AS List OF Integer = collections::sortBy([1, 3, 2], negated)
  io::print(toString(collections::get(ordered, 0)))
  RETURN 0
END FUNC
```

Order strings by length; equal lengths keep their input order:

```
IMPORT collections
IMPORT io

FUNC size(s AS String) AS Integer
  RETURN len(s)
END FUNC

FUNC main AS Integer
  LET words AS List OF String = ["pear", "fig", "kiwi", "date"]
  LET byLength AS List OF String = collections::sortBy(words, size)
  io::print(collections::get(byLength, 0))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections sort`
- `mfb man collections transform`
- `mfb man collections groupBy`
- `mfb man collections`
