# isSuperset

Test whether the first set contains every element of the second

## Synopsis

```
collections::isSuperset OF T(a AS Set OF T, b AS Set OF T) AS Boolean
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

`collections::isSuperset` returns `TRUE` when every element of `b` is also in
`a`, and `FALSE` otherwise. It is `isSubset` with the arguments swapped: it walks
the elements of `b` and returns `FALSE` as soon as `collections::contains` reports
one that is absent from `a`, returning `TRUE` if the walk finds no such element.
[[src/builtins/collections_package.mfb:__collections_isSuperset]]

`isSuperset` is **pure**: it inspects both arguments and mutates neither. Every
set is a superset of the empty set, so `isSuperset(a, Set OF T { })` is always
`TRUE`. A set is a superset of itself, and equal sets are supersets of each other.

`isSuperset` raises no user-trappable error of its own.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`isSuperset` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_isSuperset` generic and instantiated for the element
type like any other generic function. [[src/builtins/collections.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Set OF T` | The candidate superset, tested against every element of `b`. Not modified. `T` must be a comparable type. [[src/builtins/collections_package.mfb:__collections_isSuperset]] |
| `b` | `Set OF T` | The candidate subset, of the same type as `a`, whose elements are each tested for membership in `a`. Not modified. [[src/builtins/collections_package.mfb:__collections_isSuperset]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when every element of `b` is in `a` (including when `b` is empty); `FALSE` when some element of `b` is not in `a`. [[src/builtins/collections_package.mfb:__collections_isSuperset]] |

## Errors

No errors.

## Type checking

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time. [[src/builtins/collections_package.mfb:__collections_isSuperset]]

## Examples

A larger set containing a smaller one:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isSuperset(Set OF Integer { 1, 2, 3 }, Set OF Integer { 1, 2 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

A missing element makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isSuperset(Set OF Integer { 1, 2, 3 }, Set OF Integer { 1, 9 })
  io::print(toString(no))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections isSubset`
- `mfb man collections isDisjoint`
- `mfb man collections contains`
- `mfb man collections union`
- `mfb man types set`
