# symmetricDifference

Return the set of elements in exactly one of two sets

## Synopsis

```
collections::symmetricDifference OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
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

`collections::symmetricDifference` returns a new `Set OF T` holding the elements
that are in exactly one of `a` and `b` — every element of their union that is not
in their intersection. It is computed as a two-pass fold: it keeps each element of
`a` that `collections::contains` reports as absent from `b`, then adds each
element of `b` that is absent from `a`. Unlike `difference`, the operation is
symmetric: `symmetricDifference(a, b)` and `symmetricDifference(b, a)` are equal.
[[src/codegen/builtins/collections/package.mfb:__collections_symmetricDifference]]

`symmetricDifference` is **pure**: it returns a new value and mutates neither
argument. Element insertion order follows the surviving elements of `a` first,
then the surviving elements of `b`. The symmetric difference of two equal sets is
the empty set, and of a set with the empty set is a copy of that set.

`symmetricDifference` raises no user-trappable error of its own. Allocation
failure is not a trappable domain error, and the `add` it is built on is
classified infallible. [[src/builtins/mod.rs:inline_builtin_is_infallible]]

`symmetricDifference` is a generic implemented in MFBASIC source; a call is
rewritten to the internal `__collections_symmetricDifference` generic and
instantiated for the element type like any other generic function.
[[src/codegen/builtins/collections/mod.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Set OF T` | The first set. Not modified. `T` must be a comparable type. [[src/codegen/builtins/collections/package.mfb:__collections_symmetricDifference]] |
| `b` | `Set OF T` | The second set, of the same type as `a`. Not modified. [[src/codegen/builtins/collections/package.mfb:__collections_symmetricDifference]] |

## Return value

| Type | Description |
| --- | --- |
| `Set OF T` | A new set of the elements in exactly one of `a` and `b`; its length is between `0` and `len(a) + len(b)`. [[src/codegen/builtins/collections/package.mfb:__collections_symmetricDifference]] |

## Errors

No errors.

## Type checking

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time. [[src/codegen/builtins/collections/package.mfb:__collections_symmetricDifference]]

## Examples

Elements in exactly one of two sets:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET d AS Set OF Integer = collections::symmetricDifference(Set OF Integer { 1, 2, 3 }, Set OF Integer { 2, 3, 4 })
  io::print(toString(len(d)))
  RETURN 0
END FUNC
```

Two equal sets have an empty symmetric difference:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET d AS Set OF Integer = collections::symmetricDifference(Set OF Integer { 1, 2 }, Set OF Integer { 1, 2 })
  io::print(toString(len(d)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections difference`
- `mfb man collections union`
- `mfb man collections intersection`
- `mfb man types set`
- `mfb man collections`
