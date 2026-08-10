# isSubset

Test whether every element of the first set is in the second

## Synopsis

```
collections::isSubset OF T(a AS Set OF T, b AS Set OF T) AS Boolean
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/collections/mod.rs:is_collections_call]]

## Description

`collections::isSubset` returns `TRUE` when every element of `a` is also in `b`,
and `FALSE` otherwise. It walks the elements of `a` and returns `FALSE` as soon as
`collections::contains` reports one that is absent from `b`; if the walk finishes
with no such element, it returns `TRUE`.
[[src/codegen/builtins/collections/collections_package.mfb:__collections_isSubset]]

`isSubset` is **pure**: it inspects both arguments and mutates neither. The empty
set is a subset of every set, so `isSubset(Set OF T { }, b)` is always `TRUE`. A
set is a subset of itself, and equal sets are subsets of each other.

`isSubset` raises no user-trappable error of its own.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`isSubset` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_isSubset` generic and instantiated for the element type
like any other generic function. [[src/codegen/builtins/collections/mod.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Set OF T` | The candidate subset, whose elements are each tested for membership in `b`. Not modified. `T` must be a comparable type. [[src/codegen/builtins/collections/collections_package.mfb:__collections_isSubset]] |
| `b` | `Set OF T` | The candidate superset, of the same type as `a`. Not modified. [[src/codegen/builtins/collections/collections_package.mfb:__collections_isSubset]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when every element of `a` is in `b` (including when `a` is empty); `FALSE` when some element of `a` is not in `b`. [[src/codegen/builtins/collections/collections_package.mfb:__collections_isSubset]] |

## Errors

No errors.

## Type checking

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time. [[src/codegen/builtins/collections/collections_package.mfb:__collections_isSubset]]

## Examples

A smaller set contained in a larger one:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isSubset(Set OF Integer { 1, 2 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

An element outside the second set makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isSubset(Set OF Integer { 1, 9 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(no))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections isSuperset`
- `mfb man collections isDisjoint`
- `mfb man collections contains`
- `mfb man collections intersection`
- `mfb man types set`
