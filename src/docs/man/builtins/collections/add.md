# add

Return a set with one element inserted, leaving the argument unchanged

## Synopsis

```
collections::add OF T(value AS Set OF T, item AS T) AS Set OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/collections/mod.rs:is_collections_call]]

`add` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `add` built-in.
[[src/codegen/builtins/collections/mod.rs:is_native_member]]

## Description

`collections::add` returns a new `Set OF T` containing every element of `value`
plus `item`. It takes exactly two arguments; neither is optional and neither is
variadic. [[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

Insertion is **idempotent**: if an equal element is already in `value`, the
result is a set with the same elements — no duplicate is created and the length
is unchanged. When `item` is new, the result has one more element than `value`,
appended in insertion order so a later `collections::toList` places it last.
[[src/codegen/builtins/collections/func_add.rs:lower_add]]

`add` is value-semantic. The set named by `value` is unchanged; the modified set
is the returned value, and a program observes the update only through what it
does with that return value. When the compiler can prove the target is a
uniquely-owned local being reassigned — the `set = collections::add(set, x)`
shape — it may update the live buffer in place; this is an optimization only, and
the observable semantics are identical either way.

`add` is **infallible**: no path in its lowering raises a trappable domain error,
so an inline `TRAP` written on an `add` call has a dead handler. Allocation
exhaustion is not a trappable domain error in this language.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Set OF T` | The set to insert into; left unchanged. Also accepted under the name `set`. Must be a set type; passing a `List` or a scalar resolves no overload and is a compile-time error. [[src/codegen/builtins/collections/mod.rs:call_param_names]] [[src/codegen/builtins/collections/mod.rs:resolve_set_add]] |
| `item` | `T` | The element to insert. Also accepted under the name `element`. Its type must be exactly the element type `T`. [[src/codegen/builtins/collections/mod.rs:call_param_names]] [[src/codegen/builtins/collections/mod.rs:resolve_set_add]] |

## Return value

| Type | Description |
| --- | --- |
| `Set OF T` | A new set with `item` present, of the same type as `value`. Adding an element already in `value` returns a set equal to `value`. [[src/codegen/builtins/collections/mod.rs:resolve_set_add]] |

## Errors

No errors.

## Type checking

The first argument must be a `Set OF T` and the second must be exactly the
element type `T`; there is no implicit widening or conversion. A call on a
non-set first argument, or with an element type that does not match, resolves to
no overload and is rejected at compile time. Because a set requires a comparable
element type, `T` must be comparable.
[[src/codegen/builtins/collections/mod.rs:resolve_set_add]]
[[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

## Examples

Insert a new element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::add(Set OF Integer { 1, 2 }, 3)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Adding an element already present is a no-op:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::add(Set OF Integer { 1, 2 }, 2)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Build a set in a loop; the argument is never mutated, the result is:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT seen AS Set OF Integer = Set OF Integer { }
  FOR i = 1 TO 5
    seen = collections::add(seen, i MOD 2)
  NEXT
  io::print(toString(len(seen)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections remove`
- `mfb man collections contains`
- `mfb man collections toList`
- `mfb man collections union`
- `mfb man types set`
