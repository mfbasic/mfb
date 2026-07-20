# append

Return a list with one element, or every element of another list, added at the end

## Synopsis

```
collections::append OF T(value AS List OF T, item AS T) AS List OF T
collections::append OF T(value AS List OF T, item AS List OF T) AS List OF T
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

`collections::append` returns a new list whose contents are those of `value`
followed by the appended content. It takes exactly two arguments; neither is
optional and neither is variadic. [[src/builtins/collections.rs:arity]]

The second argument may be either a single element of the list's element type
`T`, or another `List OF T`. The compiler picks the overload from the static type
of that argument: an argument whose type is exactly the element type appends one
element, and an argument whose type is exactly the same list type concatenates.
Any other type is a compile-time error, because no other combination resolves.
[[src/builtins/general.rs:resolve_append]]

Internally both forms are the same operation: the appended content is wrapped as
a list when it is a single element, and the result is built by splicing that list
into `value` at index `count(value)` — the one-past-the-end position, which the
splice accepts as the append position. Existing elements keep their relative
order, and the appended content is placed after all of them in its own order.
[[src/target/shared/code/builder_collection_mutate.rs:lower_collection_append]]
[[src/target/shared/code/builder_collection_mutate.rs:lower_list_insert_collection]]

`append` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. When the compiler can prove the target is a
uniquely owned local being reassigned — the `list = collections::append(list, x)`
shape, on a non-`by_ref` local that is not the live iterable of an enclosing
`FOR EACH` — it lowers the call to an in-place grow with geometric spare
capacity, making a repeated append amortized O(1) rather than a full copy. This
is an optimization only: the observable semantics are identical either way.
[[src/target/shared/code/builder_inplace_assign.rs:try_inplace_append_assign]]
[[src/target/shared/code/builder_inplace_assign.rs:try_inplace_bulk_append_assign]]

`append` is **infallible**: no path in its lowering raises a trappable domain
error. It has no index to range-check and no lookup to miss, so it is classified
as infallible alongside `prepend` and `replace`, and an inline `TRAP` written on
an `append` call has a dead handler (the front end reports
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Allocation exhaustion is not a trappable domain
error in this language. [[src/builtins/mod.rs:inline_builtin_is_infallible]]

Appending an empty list returns a copy of `value` with the same elements in the
same order.

## Overloads

**`collections::append OF T(value AS List OF T, item AS T) AS List OF T`**

Appends a single element. The result is exactly one element longer than `value`,
with `item` as its last element.
[[src/builtins/general.rs:resolve_append]]

**`collections::append OF T(value AS List OF T, item AS List OF T) AS List OF T`**

Concatenates a second list of the same type. The result is
`len(value) + len(item)` elements long, with every element of `item` following
every element of `value`, both in their original order.
[[src/builtins/general.rs:resolve_append]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to append to; left unchanged. Also accepted under the name `list`. Must be a list type; passing a `Map` or a scalar resolves no overload and is a compile-time error. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_append]] |
| `item` | `T` or `List OF T` | The element to append, or a list of elements to concatenate. Also accepted under the name `items`. Its type must be exactly the element type `T` or exactly `List OF T`. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_append]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list with the appended content at the end, of the same type as `value`. Appending an empty list returns a list equal to `value`. [[src/builtins/general.rs:resolve_append]] |

## Errors

No errors.

## Type checking

The first argument must be a `List OF T`. The second argument must have a type
equal to the element type `T` or equal to the full list type `List OF T`; there
is no implicit widening or conversion, so appending an `Integer` to a
`List OF Float` does not resolve. A call on a non-list first argument, or with an
element type that does not match, resolves to no overload and is rejected at
compile time. [[src/builtins/general.rs:resolve_append]]

## Examples

Append a single element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::append([1, 2], 3)
  io::print(toString(len(numbers)))
  RETURN 0
END FUNC
```

Concatenate a second list:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::append([1, 2], [3, 4])
  RETURN 0
END FUNC
```

Build a list in a loop; the argument is never mutated, the result is:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT bytes AS List OF Byte = []
  FOR i = 65 TO 70
    bytes = collections::append(bytes, toByte(i))
  NEXT
  io::print(toString(len(bytes)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections prepend`
- `mfb man collections insert`
- `mfb man collections set`
- `mfb man collections removeAt`
- `mfb man collections`
