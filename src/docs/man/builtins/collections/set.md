# set

Return a collection with one element replaced, or one map key assigned

## Synopsis

```
collections::set OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T
collections::set OF K, V(value AS Map OF K TO V, index AS K, item AS V) AS Map OF K TO V
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

`collections::set` returns a new collection with one position updated. It takes
exactly three arguments; none is optional and none is variadic. The first
argument selects the overload: a `List OF T` is addressed by an `Integer` index,
and a `Map OF K TO V` is addressed by a key of type `K`.
[[src/builtins/collections.rs:arity]] [[src/builtins/general.rs:resolve_set]]

The two overloads differ in more than addressing — they differ in whether a
missing position is an error:

- For a **list**, the index must already exist. The bound is
  `0 <= index < len(value)`; the result has the same length as `value` and only
  the element at `index` differs. An index equal to the length is **not** an
  append position and raises `ErrIndexOutOfRange`, as does any negative index.
  Use `collections::append` or `collections::insert` to grow a list.
  [[src/target/shared/code/builder_collection_mutate.rs:lower_list_set_in_place]]
- For a **map**, the key need not exist. When the key is present its value is
  overwritten; when it is absent a new entry is inserted. The map overload has no
  failure path at all — it raises no domain error for any key.
  [[src/target/shared/code/builder_collection_mutate.rs:lower_map_set_in_place]]
  [[src/target/shared/code/builder_collection_mutate.rs:lower_collection_set]]

`set` is value-semantic in both overloads. The collection named by `value` is
unchanged; the updated collection is the returned value, and a program observes
the update only through what it does with that return value. When the compiler
can prove the target is a uniquely owned local being reassigned — the
`c = collections::set(c, k, v)` shape, on a non-`by_ref` local that is not the
live iterable of an enclosing `FOR EACH` — it lowers the call to an in-place
update instead of rebuilding the collection. This is an optimization only; the
observable semantics, including the list bounds check, are identical either way.
[[src/target/shared/code/builder_inplace_assign.rs:try_inplace_set_assign]]

On the general (copying) path the list overload is composed from
`removeAt(index)` followed by an insert of the replacement at the same index,
which is where its `0 <= index < len(value)` bound comes from; the map overload
is composed from `removeKey` — which is a filter and never fails on a missing
key — followed by a concatenation of the single new entry, which is why an
absent key inserts rather than raising.
[[src/target/shared/code/builder_collection_mutate.rs:lower_collection_set]]
[[src/target/shared/code/builder_collection_mutate.rs:lower_map_remove_key]]

`set` is classified **fallible** overall because of the list overload's range
check, so an inline `TRAP` on a `set` call compiles and catches that failure
rather than being reported as a dead handler. On the list path the bounds test
runs before any replacement value is materialized, so a rejected index allocates
nothing. [[src/builtins/mod.rs:inline_builtin_raw_supported]]

## Overloads

**`collections::set OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T`**

Replaces the existing element at `index`. Length is preserved and the index must
be in range; this overload can raise `ErrIndexOutOfRange`.
[[src/builtins/general.rs:resolve_set]]

**`collections::set OF K, V(value AS Map OF K TO V, index AS K, item AS V) AS Map OF K TO V`**

Associates `item` with the key, overwriting an existing entry or inserting a new
one. This overload cannot fail. Note that the key parameter is still spelled
`index` canonically; `key` is accepted as an alternate name.
[[src/builtins/general.rs:resolve_set]] [[src/builtins/collections.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` or `Map OF K TO V` | The collection to update; left unchanged. Also accepted under the name `collection`. Its shape selects the overload. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_set]] |
| `index` | `Integer` or `K` | For a list, the zero-based position to overwrite, valid in `0` through `len(value) - 1`. For a map, the key to assign, which may or may not already be present. Also accepted under the name `key`. Its type must be exactly `Integer` for a list, or exactly the map's key type `K`. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_set]] |
| `item` | `T` or `V` | The replacement element (list) or associated value (map). Its type must be exactly the list's element type `T` or the map's value type `V`. This parameter has no alternate spelling. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_set]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | For the list overload: a new list of the same type and the same length as `value`, differing only at `index`. [[src/builtins/general.rs:resolve_set]] |
| `Map OF K TO V` | For the map overload: a new map of the same type as `value` in which the key maps to `item`. Its size is unchanged when the key was already present and one larger when it was not. [[src/builtins/general.rs:resolve_set]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | **List overload only.** `index` is negative, or `index` is greater than or equal to `len(value)`. The map overload raises no error for any key, present or absent. [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] [[src/target/shared/code/builder_collection_mutate.rs:lower_list_set_in_place]] |

## Type checking

The list overload requires an `Integer` second argument and a third argument of
exactly the element type `T`. The map overload requires a second argument of
exactly the key type `K` and a third of exactly the value type `V`. There is no
implicit widening or conversion in any position, so a `Map OF String TO Float`
does not accept an `Integer` value. A first argument that is neither a list nor a
map, or any type mismatch, resolves to no overload and is rejected at compile
time; the list index range is a runtime check, not a compile-time one.
[[src/builtins/general.rs:resolve_set]]

## Examples

Replace an existing list element:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::set([1, 2, 3], 1, 9)
  RETURN 0
END FUNC
```

Insert and then overwrite a map key — neither call can fail:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT scores AS Map OF String TO Integer = Map OF String TO Integer {}
  scores = collections::set(scores, "Ada", 10)
  scores = collections::set(scores, "Ada", 20)
  io::print(toString(collections::get(scores, "Ada")))
  RETURN 0
END FUNC
```

A list index equal to the length is out of range, not an append:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::set([1, 2], 2, 9) TRAP(e)
    io::print(e.message)
    RECOVER collections::append([1, 2], 9)
  END TRAP
  RETURN 0
END FUNC
```

## See also

- `mfb man collections get`
- `mfb man collections getOr`
- `mfb man collections insert`
- `mfb man collections append`
- `mfb man collections removeAt`
- `mfb man collections removeKey`
- `mfb man collections`
