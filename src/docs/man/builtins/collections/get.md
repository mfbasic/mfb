# get

Read a list item by index or a map value by key.

## Synopsis

```
collections::get OF T(value AS List OF T, index AS Integer) AS T
collections::get OF K, V(value AS Map OF K TO V, key AS K) AS V
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

`get` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `get` built-in.
[[src/builtins/collections.rs:is_native_member]]

## Description

`collections::get` reads one element out of a collection. The collection itself
is neither copied nor mutated: the lowering stores only a handle to it, walks
its lookup table, and materializes just the selected payload.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_get]]

The value returned is **owned** by the caller. Scalars are returned by value and
a `String` payload is materialized fresh, while a composite payload stored
inline in the collection's data region is copied into a standalone arena block
before it is handed back, so binding, storing, and freeing the result cannot
disturb the source collection.
[[src/target/shared/code/builder_collection_queries.rs:materialize_owned_element]]

`get` is the only fallible member of this group. It reports a missing element as
a trappable domain error rather than substituting anything, and it is
raw-supported, so an inline `TRAP` on a `collections::get` call catches the real
runtime error. When a fallback value is more convenient than an error, use
`collections::getOr`; when only presence matters, use `collections::hasKey`.
[[src/builtins/mod.rs:inline_builtin_raw_supported]]

For the map overload, key comparison is a comparison of the stored key payload:
fixed-width keys compare their raw 64-bit (or 32-bit, or single-byte) stored
bits and `String` keys compare length first and then bytes. A `Float` key is
therefore matched bit-for-bit, so `NaN` never matches any key and `-0.0` does
not match a stored `0.0`.
[[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_match_branch]]

Map lookup for the common key types `String`, `Integer`, `Float`, `Fixed`,
`Byte`, and `Boolean` goes through the map's hash bucket index; other key types
fall back to a linear scan of the entry table. This is a performance difference
only — both paths select the same entry and raise the same error when the key is
absent. [[src/target/shared/code/builder_collection_query.rs:map_key_probe_eligible]]

## Overloads

**`collections::get OF T(value AS List OF T, index AS Integer) AS T`**

Returns the item stored at zero-based `index`. The index is bounds-checked
against the list's element count before any payload is read: `index < 0` and
`index >= len(value)` both raise `ErrIndexOutOfRange`, so the valid range is
`0` through `len(value) - 1` and every index into an empty list fails.
[[src/target/shared/code/builder_collection_query.rs:lower_list_get]]

**`collections::get OF K, V(value AS Map OF K TO V, key AS K) AS V`**

Returns the value stored under `key`. A key that is not present raises
`ErrNotFound`; the map overload has no notion of an out-of-range key.
[[src/target/shared/code/builder_collection_query.rs:lower_map_get]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` or `Map OF K TO V` | The collection to read from. Also accepted under the name `collection`. Not copied and not mutated. [[src/builtins/collections.rs:call_param_names]] |
| `index` | `Integer` or `K` | The zero-based list index, or the map key. Also accepted under the name `key`. The spelling table is per position rather than per overload, so `index` and `key` are both accepted names under *either* overload — a map lookup may be written `collections::get(value := m, index := k)` and a list read may be written `collections::get(value := xs, key := 0)`. [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `T` (list overload) or `V` (map overload) | The selected element, as an owned value the caller may bind, store, and free. The list overload returns the list element type; the map overload returns the map value type. [[src/builtins/general.rs:resolve_get]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | List overload only: `index` is negative, or is greater than or equal to the number of items in `value`. [[src/target/shared/code/builder_collection_query.rs:lower_list_get]] |
| `77050004` | `ErrNotFound` | Map overload only: `value` has no entry whose key matches `key`. [[src/target/shared/code/builder_collection_query.rs:lower_map_get]] |

## Type checking

`collections::get` takes exactly two arguments.
[[src/builtins/collections.rs:arity]]

If the first argument is a `List OF T`, the second must be exactly `Integer` and
the call has type `T`. Otherwise the first argument must be a `Map OF K TO V`,
the second must be exactly the map key type `K`, and the call has type `V`. Any
other combination — a non-collection first argument, a non-`Integer` list index,
or a map key whose type is not `K` — is a compile-time type error.
[[src/builtins/general.rs:resolve_get]]

## Examples

Read a list item by index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30]
  io::print(toString(collections::get(numbers, 0)))
  RETURN 0
END FUNC
```

Read a map value by key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  io::print(toString(collections::get(ages, "Ada")))
  RETURN 0
END FUNC
```

Guard the lookup so the missing-key error cannot be raised:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  IF collections::hasKey(ages, "Grace") THEN
    io::print(toString(collections::get(ages, "Grace")))
  END IF
  RETURN 0
END FUNC
```

## See also

- `mfb man collections getOr`
- `mfb man collections hasKey`
- `mfb man collections set`
- `mfb man collections contains`
- `mfb man collections`
