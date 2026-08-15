# union

Return the set of elements present in either of two sets

## Synopsis

```
collections::union OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
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

`collections::union` returns a new `Set OF T` holding every element that is in
`a`, in `b`, or in both. It starts from the elements of `a` and adds each element
of `b`; because `collections::add` is idempotent, an element already present is
not duplicated, so the result contains each distinct element exactly once.
[[src/codegen/builtins/collections/package.mfb:__collections_union]]

`union` is **pure**: it returns a new value and mutates neither argument. Element
insertion order follows the elements of `a` first, then the elements of `b` that
were not already in `a`. The union of a set with the empty set is a copy of that
set, and the union of two equal sets is a set equal to either one.

`union` raises no user-trappable error of its own. It allocates while building
the result, but allocation failure is not a trappable domain error, and the
`add` it is built on is classified infallible for exactly that reason.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`union` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_union` generic and instantiated for the element type like
any other generic function. [[src/codegen/builtins/collections/mod.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Set OF T` | The first set. Not modified. `T` must be a comparable type. [[src/codegen/builtins/collections/package.mfb:__collections_union]] |
| `b` | `Set OF T` | The second set, of the same type as `a`. Not modified. [[src/codegen/builtins/collections/package.mfb:__collections_union]] |

## Return value

| Type | Description |
| --- | --- |
| `Set OF T` | A new set containing every element of `a` and `b`; its length is between `max(len(a), len(b))` and `len(a) + len(b)`. [[src/codegen/builtins/collections/package.mfb:__collections_union]] |

## Errors

No errors.

## Type checking

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time. [[src/codegen/builtins/collections/package.mfb:__collections_union]]

## Examples

Combine two sets:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET u AS Set OF Integer = collections::union(Set OF Integer { 1, 2 }, Set OF Integer { 2, 3 })
  io::print(toString(len(u)))
  RETURN 0
END FUNC
```

Union with an empty set is a copy:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET u AS Set OF Integer = collections::union(Set OF Integer { 4, 5 }, Set OF Integer { })
  io::print(toString(len(u)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections intersection`
- `mfb man collections difference`
- `mfb man collections symmetricDifference`
- `mfb man collections add`
- `mfb man types set`
