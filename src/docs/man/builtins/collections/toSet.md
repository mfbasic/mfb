# toSet

Build a set from the distinct elements of a list

## Synopsis

```
collections::toSet OF T(value AS List OF T) AS Set OF T
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

`collections::toSet` returns a new `Set OF T` containing every distinct element
of the list `value`. It folds over `value` in order and adds each element to a
fresh set; because `collections::add` is idempotent, a repeated element is stored
only once, so the result holds each distinct element exactly once.
[[src/builtins/collections_package.mfb:__collections_toSet]]

`toSet` is **pure**: it returns a new value and does not mutate `value`. Element
insertion order follows first occurrence in the list, so `toSet([2, 1, 2, 3])`
holds `2`, `1`, `3` in that order. Converting a list that is already free of
duplicates yields a set with the same elements; converting the empty list yields
the empty set.

`toSet` raises no user-trappable error of its own. It allocates while building the
result, but allocation failure is not a trappable domain error, and the `add` it
is built on is classified infallible for exactly that reason.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`toSet` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_toSet` generic and instantiated for the element type like
any other generic function. [[src/builtins/collections.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to draw elements from. Not modified. `T` must be a comparable type, since a `Set OF T` requires a comparable element. [[src/builtins/collections_package.mfb:__collections_toSet]] |

## Return value

| Type | Description |
| --- | --- |
| `Set OF T` | A new set of the list's distinct elements, in first-occurrence order; its length is between `0` and `len(value)`. [[src/builtins/collections_package.mfb:__collections_toSet]] |

## Errors

No errors.

## Type checking

The argument must be a `List OF T` whose element type `T` is comparable (every
`Set OF T` requires it). A call on a non-list argument, or on a list whose element
type is not comparable, does not resolve and is rejected at compile time.
[[src/builtins/collections_package.mfb:__collections_toSet]]

## Examples

Collapse a list's duplicates into a set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::toSet([5, 5, 6, 7, 6])
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Round-trip a set through a list and back:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET original AS Set OF String = Set OF String { "a", "b", "c" }
  LET roundTripped AS Set OF String = collections::toSet(collections::toList(original))
  io::print(toString(len(roundTripped)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections toList`
- `mfb man collections union`
- `mfb man collections distinct`
- `mfb man types set`
