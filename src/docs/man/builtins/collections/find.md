# find

Return the index of the first matching element or contiguous sublist in a list

## Synopsis

```
collections::find OF T(value AS List OF T, item AS T) AS Integer
collections::find OF T(value AS List OF T, item AS T, start AS Integer) AS Integer
collections::find OF T(value AS List OF T, item AS List OF T) AS Integer
collections::find OF T(value AS List OF T, item AS List OF T, start AS Integer) AS Integer
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

`collections::find` scans `value` forward from `start` and returns the
zero-based index of the first match. It is a **native** member: the compiler
emits the search loop directly rather than instantiating an MFBASIC generic.
[[src/builtins/collections.rs:is_native_member]]
[[src/target/shared/code/builder_search.rs:lower_find]]

This page documents the `List` form only. `collections::find` accepts nothing
but a `List` as its first argument; the `String` search of the same name lives in
`strings::`. [[src/builtins/general.rs:resolve_find_list]]

Two searches share the name, chosen by the type of the second argument. When it
has the element type `T`, `find` performs an **element search**. When it has the
same `List OF T` type as `value`, `find` performs a **contiguous sublist
search**. The element form is tested first, so for a list of lists — where the
element type is itself a `List` — a second argument of that element type is read
as an element search. Any other second-argument type fails to resolve at compile
time. [[src/builtins/general.rs:resolve_find_list]]
[[src/target/shared/code/builder_search.rs:lower_find]]

`start` is optional. When it is omitted the search begins at index 0; the
lowering supplies that default itself, so an omitted `start` and an explicit `0`
behave identically. [[src/builtins/collections.rs:arity]]
[[src/target/shared/code/builder_search.rs:lower_find]]

`start` is validated before anything is compared. A negative `start`, or a
`start` greater than the length of `value`, fails with `ErrIndexOutOfRange`. A
`start` exactly equal to the length is **valid**: it selects an empty search
range, which yields `ErrNotFound` for an element search and, for a sublist
search with an empty needle, the index `start` itself.
[[src/target/shared/code/builder_search.rs:lower_list_find_item]]
[[src/target/shared/code/builder_search.rs:lower_list_find_sublist]]

When no match exists at or after `start`, `find` fails with `ErrNotFound`. It
never returns a sentinel such as `-1`; a search that may legitimately come up
empty needs a `TRAP`, or `collections::contains` if only the yes/no answer is
wanted. [[src/target/shared/code/builder_search.rs:lower_list_find_item]]

Element equality is decided on the stored payload. `String` elements compare by
length and then byte for byte; `Integer`, `Float`, `Fixed`, and `Money` elements
compare as their stored 64-bit pattern, so `Float` matching is bit-exact and a
`NaN` never matches itself; `Boolean`, `Byte`, and `Scalar` compare as their
narrower stored value; record elements compare field by field. A nested
collection that is stored as a handle rather than inlined compares by identity,
not by contents.
[[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_matches_value_branch]]
[[src/target/shared/code/builder_collection_layout.rs:is_pointer_collection_payload_type]]

`value` is neither modified nor consumed, and no new collection is allocated.

## Overloads

**`collections::find OF T(value AS List OF T, item AS T[, start AS Integer]) AS Integer`**

Element search. Scans indices `start`, `start + 1`, … and returns the first
index whose element equals `item`. Fails with `ErrNotFound` when no element at
or after `start` matches — including whenever `start` equals the length of
`value`. [[src/target/shared/code/builder_search.rs:lower_list_find_item]]

**`collections::find OF T(value AS List OF T, item AS List OF T[, start AS Integer]) AS Integer`**

Contiguous sublist search. Returns the lowest index `i >= start` at which the
whole of `item` appears as a run of consecutive elements of `value`; the
candidate is abandoned as soon as one element differs, and the scan stops with
`ErrNotFound` once `i` plus the needle's length would run past the end of
`value`. An **empty** `item` is a special case handled before the scan: it
matches immediately and returns `start`, for any `start` in the valid range up
to and including the length of `value`.
[[src/target/shared/code/builder_search.rs:lower_list_find_sublist]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to search. Must be a `List`; a `String` first argument selects `strings::find` instead. Also accepted under the name `list`. [[src/builtins/general.rs:resolve_find_list]] [[src/builtins/collections.rs:call_param_names]] |
| `item` | `T` or `List OF T` | What to look for: an element of type `T` for an element search, or a list of the same type as `value` for a contiguous sublist search. Also accepted under the name `needle`. [[src/builtins/general.rs:resolve_find_list]] [[src/builtins/collections.rs:call_param_names]] |
| `start` | `Integer` | Optional zero-based index at which the scan begins; defaults to `0` when omitted. Must be in the range `0` through the length of `value` inclusive. There is no alternate name for this parameter. [[src/builtins/collections.rs:arity]] [[src/target/shared/code/builder_search.rs:lower_find]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The zero-based index of the first match at or after `start`: the index of the matching element, or the index at which the matching run begins. For an empty sublist needle, `start` itself. A failed search raises instead of returning a value. [[src/builtins/general.rs:resolve_find_list]] [[src/target/shared/code/builder_search.rs:lower_list_find_sublist]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `start` is negative, or `start` is greater than the length of `value`. Checked before any comparison, for both overloads. [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] [[src/target/shared/code/builder_search.rs:lower_list_find_item]] |
| `77050004` | `ErrNotFound` | No element equal to `item` exists at or after `start`, or no occurrence of the sublist `item` begins at or after `start`. [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] [[src/target/shared/code/builder_search.rs:lower_list_find_sublist]] |

## Type checking

`T` is the element type of `value`, which must be a `List`. The second argument
must be either exactly `T` or exactly the type of `value`; nothing else
resolves, and the `T` case wins when both would match. A supplied `start` must
be exactly `Integer` — no other numeric type is accepted, and the whole call
takes two or three arguments.
[[src/builtins/general.rs:resolve_find_list]] [[src/builtins/collections.rs:arity]]

## Examples

Find an element, with and without a starting index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30, 20]
  io::print(toString(collections::find(numbers, 20)))
  io::print(toString(collections::find(numbers, 20, 2)))
  RETURN 0
END FUNC
```

Find a contiguous sublist:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30, 20]
  LET needle AS List OF Integer = [20, 30]
  io::print(toString(collections::find(numbers, needle)))
  RETURN 0
END FUNC
```

Handle a missing element instead of letting `ErrNotFound` propagate:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  LET index AS Integer = collections::find(numbers, 99) TRAP(e)
    io::print("absent: " & e.message)
    RECOVER -1
  END TRAP
  io::print(toString(index))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections contains`
- `mfb man collections findIndex`
- `mfb man collections mid`
- `mfb man collections get`
- `mfb man strings find`
- `mfb man collections`
