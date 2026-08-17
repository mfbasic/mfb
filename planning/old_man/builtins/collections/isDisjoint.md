# isDisjoint

Test whether two sets share no element

## Synopsis

```
collections::isDisjoint OF T(a AS Set OF T, b AS Set OF T) AS Boolean
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/codegen/registry/mod.rs:owning_package]]

## Description

`collections::isDisjoint` returns `TRUE` when `a` and `b` have no element in
common, and `FALSE` otherwise. It walks the elements of `a` and returns `FALSE`
as soon as `collections::contains` reports one that is also in `b`; if the walk
finds no shared element, it returns `TRUE`. Equivalently, two sets are disjoint
exactly when their intersection is empty.
[[src/codegen/builtins/collections/package.mfb:__collections_isDisjoint]]

`isDisjoint` is **pure**: it inspects both arguments and mutates neither. The
empty set is disjoint from every set, so a call with an empty argument is always
`TRUE`. The relation is symmetric: `isDisjoint(a, b)` equals `isDisjoint(b, a)`.

`isDisjoint` raises no user-trappable error of its own.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`isDisjoint` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_isDisjoint` generic and instantiated for the element
type like any other generic function. [[src/codegen/builtins/collections/mod.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Set OF T` | The first set, walked element by element. Not modified. `T` must be a comparable type. [[src/codegen/builtins/collections/package.mfb:__collections_isDisjoint]] |
| `b` | `Set OF T` | The second set, of the same type as `a`, tested for shared membership. Not modified. [[src/codegen/builtins/collections/package.mfb:__collections_isDisjoint]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `a` and `b` share no element (including when either is empty); `FALSE` when they share at least one element. [[src/codegen/builtins/collections/package.mfb:__collections_isDisjoint]] |

## Errors

No errors.

## Type checking

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time. [[src/codegen/builtins/collections/package.mfb:__collections_isDisjoint]]

## Examples

Two sets with no common element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET yes AS Boolean = collections::isDisjoint(Set OF Integer { 1, 2 }, Set OF Integer { 3, 4 })
  io::print(toString(yes))
  RETURN 0
END FUNC
```

A shared element makes it false:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET no AS Boolean = collections::isDisjoint(Set OF Integer { 1, 2 }, Set OF Integer { 2, 3 })
  io::print(toString(no))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections intersection`
- `mfb man collections isSubset`
- `mfb man collections isSuperset`
- `mfb man collections contains`
- `mfb man types set`
