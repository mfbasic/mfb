# prepend

Return a list with one element added at the start

## Synopsis

```
collections::prepend OF T(value AS List OF T, item AS T) AS List OF T
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

`collections::prepend` returns a new list whose first element is `item` and whose
remaining elements are those of `value` in their original order. The result is
always exactly one element longer than `value`. It takes exactly two arguments;
neither is optional and neither is variadic.
[[src/builtins/collections.rs:arity]]

Unlike `collections::append`, `prepend` has **only** the single-element form.
There is no list-into-list overload: the second argument must have exactly the
element type `T`, and passing another `List OF T` resolves no overload and is a
compile-time error. The lowering rejects a list-typed item explicitly as well.
To place a whole list in front of another, use `collections::append` with the
operands reversed — `collections::append(front, back)`.
[[src/builtins/general.rs:resolve_prepend]]
[[src/target/shared/code/collection_mutate.rs:lower_collection_prepend]]

Internally the element is wrapped as a one-element list and spliced into `value`
at index `0`, so the operation is the index-`0` case of the same splice that
backs `append` and `insert`.
[[src/target/shared/code/list_mutate.rs:lower_list_insert_collection]]

`prepend` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value. When the compiler can prove the target is a uniquely
owned local being reassigned — the `list = collections::prepend(list, x)` shape,
on a non-`by_ref` local that is not the live iterable of an enclosing `FOR EACH` —
it lowers the call to an in-place shift-and-insert with geometric spare capacity
instead of a full copy. This is an optimization only; the observable semantics
are identical either way. Note that prepending must shift every existing lookup
entry right by one, so a repeated prepend stays O(n) per call even on the
in-place path, unlike `append`.
[[src/target/shared/code/builder_inplace_assign.rs:try_inplace_prepend_assign]]
[[src/target/shared/code/list_mutate.rs:lower_list_prepend_in_place]]

`prepend` is **infallible**: no path in its lowering raises a trappable domain
error. It has no index to range-check and no lookup to miss, so it is classified
as infallible alongside `append` and `replace`, and an inline `TRAP` written on a
`prepend` call has a dead handler (the front end reports
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Allocation exhaustion is not a trappable domain
error in this language. [[src/builtins/mod.rs:inline_builtin_is_infallible]]

Prepending to an empty list yields a one-element list.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to prepend to; left unchanged. Also accepted under the name `list`. Must be a list type; passing a `Map` or a scalar resolves no overload and is a compile-time error. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_prepend]] |
| `item` | `T` | The single element to place at the front. Its type must be exactly the list's element type `T`; a `List OF T` is not accepted. This parameter has no alternate spelling. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_prepend]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the same type as `value`, one element longer, with `item` at index `0` and the elements of `value` shifted up by one. [[src/builtins/general.rs:resolve_prepend]] |

## Errors

No errors.

## Type checking

The first argument must be a `List OF T` and the second must have exactly the
element type `T`. There is no implicit widening or conversion, so prepending an
`Integer` to a `List OF Float` does not resolve. A call on a non-list first
argument, with a mismatched element type, or with a `List OF T` second argument,
resolves to no overload and is rejected at compile time.
[[src/builtins/general.rs:resolve_prepend]]

## Examples

Add an element to the front:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::prepend([2, 3], 1)
  RETURN 0
END FUNC
```

Build a reversed list by prepending in a loop:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT reversed AS List OF Integer = []
  FOR i = 1 TO 5
    reversed = collections::prepend(reversed, i)
  NEXT
  io::print(toString(collections::get(reversed, 0)))
  RETURN 0
END FUNC
```

Put a whole list in front — use `append` with the operands reversed, because
`prepend` has no list overload:

```
IMPORT collections

FUNC main AS Integer
  LET joined AS List OF Integer = collections::append([1, 2], [3, 4])
  RETURN 0
END FUNC
```

## See also

- `mfb man collections append`
- `mfb man collections insert`
- `mfb man collections set`
- `mfb man collections removeAt`
- `mfb man collections`
