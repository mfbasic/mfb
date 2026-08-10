# difference

Return the set of elements in the first set but not the second

## Synopsis

```
collections::difference OF T(a AS Set OF T, b AS Set OF T) AS Set OF T
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

`collections::difference` returns a new `Set OF T` holding the elements that are
in `a` but not in `b`. It walks the elements of `a` and keeps each one that
`collections::contains` reports as **absent** from `b`, so the result is `a` with
every element of `b` removed. The operation is asymmetric:
`difference(a, b)` and `difference(b, a)` are generally different sets.
[[src/codegen/builtins/collections/collections_package.mfb:__collections_difference]]

`difference` is **pure**: it returns a new value and mutates neither argument.
Surviving elements keep the insertion order they had in `a`. The difference of a
set and the empty set is a copy of that set; the difference of a set with itself
is the empty set.

`difference` raises no user-trappable error of its own. Allocation failure is not
a trappable domain error, and the `add` it is built on is classified infallible.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`difference` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_difference` generic and instantiated for the element
type like any other generic function. [[src/codegen/builtins/collections/mod.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Set OF T` | The set to subtract from, walked to decide element order. Not modified. `T` must be a comparable type. [[src/codegen/builtins/collections/collections_package.mfb:__collections_difference]] |
| `b` | `Set OF T` | The set whose elements are removed from `a`, of the same type as `a`. Not modified. [[src/codegen/builtins/collections/collections_package.mfb:__collections_difference]] |

## Return value

| Type | Description |
| --- | --- |
| `Set OF T` | A new set of the elements of `a` that are not in `b`; its length is between `0` and `len(a)`. [[src/codegen/builtins/collections/collections_package.mfb:__collections_difference]] |

## Errors

No errors.

## Type checking

Both arguments must be the same `Set OF T`. `T` is inferred from the element type
and **must be comparable**, which every `Set OF T` already requires. A call whose
arguments are not both sets of the same element type does not resolve and is
rejected at compile time. [[src/codegen/builtins/collections/collections_package.mfb:__collections_difference]]

## Examples

Elements of the first set not in the second:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET only AS Set OF Integer = collections::difference(Set OF Integer { 1, 2, 3 }, Set OF Integer { 2 })
  io::print(toString(len(only)))
  RETURN 0
END FUNC
```

Difference is asymmetric:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET d AS Set OF Integer = collections::difference(Set OF Integer { 2 }, Set OF Integer { 1, 2, 3 })
  io::print(toString(len(d)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections symmetricDifference`
- `mfb man collections intersection`
- `mfb man collections union`
- `mfb man collections remove`
- `mfb man types set`
