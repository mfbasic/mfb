# toList

Return the elements of a set as a list, in insertion order

## Synopsis

```
collections::toList OF T(value AS Set OF T) AS List OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/collections/mod.rs:is_collections_call]]

`toList` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `toList` built-in.
[[src/codegen/builtins/collections/mod.rs:is_native_member]]

## Description

`collections::toList` returns a new `List OF T` holding every element of the set
`value` exactly once, in the set's stable insertion order. It takes exactly one
argument, which is neither optional nor variadic.
[[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

The set is neither copied for the caller nor mutated: the result is a freshly
built list. Because a set already holds each element at most once, the resulting
list has no duplicates and its length equals `len(value)`. An empty set yields an
empty list. [[src/codegen/builtins/collections/func_to_list.rs:lower_to_list]]

`toList` is **infallible**: no path in its lowering raises a trappable domain
error, so an inline `TRAP` written on a `toList` call has a dead handler.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Set OF T` | The set whose elements are listed, in insertion order. Also accepted under the name `set`. Must be a set type; passing a `List` or a scalar resolves no overload and is a compile-time error. [[src/codegen/builtins/collections/mod.rs:call_param_names]] [[src/codegen/registry/mod.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list with the elements of `value` in insertion order; its length equals `len(value)`, and it holds no duplicates. [[src/codegen/registry/mod.rs:resolve_call]] |

## Errors

No errors.

## Type checking

The single argument must be a `Set OF T`; a `List`, a `Map`, or any other value
resolves no overload and is rejected at compile time. The result type is
`List OF T` for the set's element type `T`.
[[src/codegen/registry/mod.rs:resolve_call]]
[[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

## Examples

List the elements of a set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET elems AS List OF Integer = collections::toList(Set OF Integer { 3, 1, 2 })
  io::print(toString(len(elems)))
  RETURN 0
END FUNC
```

Duplicate elements never appear in the listed result:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT s AS Set OF Integer = Set OF Integer { 1, 2 }
  s = collections::add(s, 2)
  io::print(toString(len(collections::toList(s))))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections add`
- `mfb man collections remove`
- `mfb man collections contains`
- `mfb man types set`
- `mfb man types list`
