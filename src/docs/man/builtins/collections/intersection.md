# intersection

Return the set of elements present in both of two sets

## Synopsis

```
collections::intersection OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
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

`collections::intersection` returns a new `Set OF T` holding exactly the elements
that are in both `a` and `b`. It walks the elements of `a` and keeps each one
that `collections::contains` reports as present in `b`, so an element only
survives when it appears in both sets.
[[src/codegen/builtins/collections/package.mfb:__collections_intersection]]

`intersection` is **pure**: it returns a new value and mutates neither argument.
Surviving elements keep the insertion order they had in `a`. The intersection of
disjoint sets is the empty set, and the intersection of a set with itself is a
set equal to it.

`intersection` raises no user-trappable error of its own. Allocation failure is
not a trappable domain error, and the `add` it is built on is classified
infallible. [[src/builtins/mod.rs:inline_builtin_is_infallible]]

`intersection` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_intersection` generic and instantiated for the
element type like any other generic function.
[[src/codegen/builtins/collections/mod.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Set OF T` | The first set, walked to decide element order. Not modified. `T` must be a comparable type. [[src/codegen/builtins/collections/package.mfb:__collections_intersection]] |
| `b` | `Set OF T` | The second set, of the same type as `a`, tested for membership. Not modified. [[src/codegen/builtins/collections/package.mfb:__collections_intersection]] |

## Return value

| Type | Description |
| --- | --- |
| `Set OF T` | A new set of the elements common to `a` and `b`; its length is between `0` and `min(len(a), len(b))`. [[src/codegen/builtins/collections/package.mfb:__collections_intersection]] |

## Errors

No errors.

## Type checking

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time. [[src/codegen/builtins/collections/package.mfb:__collections_intersection]]

## Examples

Elements common to two sets:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET both AS Set OF Integer = collections::intersection(Set OF Integer { 1, 2, 3 }, Set OF Integer { 2, 3, 4 })
  io::print(toString(len(both)))
  RETURN 0
END FUNC
```

Disjoint sets intersect to the empty set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET both AS Set OF Integer = collections::intersection(Set OF Integer { 1, 2 }, Set OF Integer { 3, 4 })
  io::print(toString(len(both)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections union`
- `mfb man collections difference`
- `mfb man collections isDisjoint`
- `mfb man collections contains`
- `mfb man types set`
