# remove

Return a set with one element removed, leaving the argument unchanged

## Synopsis

```
collections::remove OF T(value AS Set OF T, item AS T) AS Set OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/collections/mod.rs:is_collections_call]]

`remove` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `remove` built-in.
[[src/codegen/builtins/collections/mod.rs:is_native_member]]

## Description

`collections::remove` returns a new `Set OF T` containing every element of
`value` except `item`. It takes exactly two arguments; neither is optional and
neither is variadic. [[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

Removal is a **no-op when the element is absent**: if no element equal to `item`
is in `value`, the result is a set with the same elements and the same length.
When `item` is present, the result has exactly one fewer element and the
remaining elements keep their relative insertion order.
[[src/target/shared/code/collection_mutate.rs:lower_set_remove]]

`remove` is value-semantic. The set named by `value` is unchanged; the modified
set is the returned value, and a program observes the update only through what it
does with that return value. When the compiler can prove the target is a
uniquely-owned local being reassigned — the `set = collections::remove(set, x)`
shape — it may update the live buffer in place; this is an optimization only, and
the observable semantics are identical either way.

`remove` is **infallible**: removing an absent element is defined as a no-op
rather than a failure, so no path raises a trappable domain error and an inline
`TRAP` written on a `remove` call has a dead handler.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Set OF T` | The set to remove from; left unchanged. Also accepted under the name `set`. Must be a set type; passing a `List` or a scalar resolves no overload and is a compile-time error. [[src/codegen/builtins/collections/mod.rs:call_param_names]] [[src/codegen/builtins/collections/mod.rs:resolve_set_remove]] |
| `item` | `T` | The element to remove. Also accepted under the name `element`. Its type must be exactly the element type `T`. [[src/codegen/builtins/collections/mod.rs:call_param_names]] [[src/codegen/builtins/collections/mod.rs:resolve_set_remove]] |

## Return value

| Type | Description |
| --- | --- |
| `Set OF T` | A new set without `item`, of the same type as `value`. Removing an element not in `value` returns a set equal to `value`. [[src/codegen/builtins/collections/mod.rs:resolve_set_remove]] |

## Errors

No errors.

## Type checking

The first argument must be a `Set OF T` and the second must be exactly the
element type `T`; there is no implicit widening or conversion. A call on a
non-set first argument, or with an element type that does not match, resolves to
no overload and is rejected at compile time. Because a set requires a comparable
element type, `T` must be comparable.
[[src/codegen/builtins/collections/mod.rs:resolve_set_remove]]
[[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

## Examples

Remove a present element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::remove(Set OF Integer { 1, 2, 3 }, 2)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Removing an absent element is a no-op:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::remove(Set OF Integer { 1, 2 }, 9)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections add`
- `mfb man collections contains`
- `mfb man collections toList`
- `mfb man collections difference`
- `mfb man types set`
