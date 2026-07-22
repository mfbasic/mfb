# getOr

Read a list item or map value, returning a supplied default when it is absent.

## Synopsis

```
collections::getOr OF T(value AS List OF T, index AS Integer, default AS T) AS T
collections::getOr OF K, V(value AS Map OF K TO V, key AS K, default AS V) AS V
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

`getOr` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `getOr` built-in.
[[src/builtins/collections.rs:is_native_member]]

## Description

`collections::getOr` is the total counterpart of `collections::get`. It performs
the same lookup, but instead of raising a domain error when the element is
missing it returns `default`. It raises no trappable error at all, which is
precisely the difference between the two: an inline `TRAP` on a
`collections::getOr` call has a dead handler.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

The collection is neither copied nor mutated; only the selected payload is
materialized. [[src/target/shared/code/builder_collection_queries.rs:lower_collection_get_or]]

Both the found path and the default path return an **owned** value. When the
element type is `String`, the supplied `default` is copied into a fresh owned
string on the fallback path rather than being returned as a borrow, so the
result can be bound and freed identically no matter which path ran. A composite
payload read out of the collection is likewise copied into a standalone block
before it is returned.
[[src/target/shared/code/builder_collection_query.rs:lower_map_get_or]]
[[src/target/shared/code/builder_collection_queries.rs:materialize_owned_element]]

`default` is an ordinary argument expression, so it is evaluated before the
lookup runs, whether or not it ends up being used.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_get_or]]

For the map overload, key comparison is a comparison of the stored key payload:
fixed-width keys compare their raw stored bits and `String` keys compare length
and then bytes. A `Float` key is matched bit-for-bit, so `NaN` never matches and
`-0.0` does not match a stored `0.0`; such a lookup simply yields `default`.
[[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_match_branch]]

Map lookup for the common key types `String`, `Integer`, `Float`, `Fixed`,
`Byte`, and `Boolean` goes through the map's hash bucket index — the same probe
`collections::get` uses — with `default` substituted on the probe's not-found
branch; other key types fall back to a linear scan of the entry table. This is
a performance difference only — both paths select the same entry and yield the
same `default` when the key is absent.
[[src/target/shared/code/builder_collection_query.rs:map_key_probe_eligible]]

## Overloads

**`collections::getOr OF T(value AS List OF T, index AS Integer, default AS T) AS T`**

Returns the item at zero-based `index`, or `default` when `index` is negative or
is greater than or equal to `len(value)`. Every index into an empty list yields
`default`. [[src/target/shared/code/builder_collection_query.rs:lower_list_get_or]]

**`collections::getOr OF K, V(value AS Map OF K TO V, key AS K, default AS V) AS V`**

Returns the value stored under `key`, or `default` when the map has no entry for
that key. [[src/target/shared/code/builder_collection_query.rs:lower_map_get_or]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` or `Map OF K TO V` | The collection to read from. Also accepted under the name `collection`. Not copied and not mutated. [[src/builtins/collections.rs:call_param_names]] |
| `index` | `Integer` or `K` | The zero-based list index, or the map key. Also accepted under the name `key`. The spelling table is per position rather than per overload, so both names are accepted under either overload. [[src/builtins/collections.rs:call_param_names]] |
| `default` | `T` or `V` | The value to return when the element is absent. Also accepted under the name `fallback`. Must be exactly the list element type for the list overload, or the map value type for the map overload. Always evaluated. [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `T` (list overload) or `V` (map overload) | The selected element when it exists, otherwise `default`. Owned by the caller on both paths. [[src/builtins/general.rs:resolve_get_or]] |

## Errors

No errors.

## Type checking

`collections::getOr` takes exactly three arguments.
[[src/builtins/collections.rs:arity]]

If the first argument is a `List OF T`, the second must be exactly `Integer`,
the third must be exactly `T`, and the call has type `T`. Otherwise the first
argument must be a `Map OF K TO V`, the second must be exactly `K`, the third
must be exactly `V`, and the call has type `V`. A `default` whose type differs
from the element or value type is a compile-time type error — there is no
widening or conversion.
[[src/builtins/general.rs:resolve_get_or]]

## Examples

Read a list item with a fallback for an out-of-range index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30]
  io::print(toString(collections::getOr(numbers, 99, 0)))
  RETURN 0
END FUNC
```

Read a map value with a fallback for a missing key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  io::print(toString(collections::getOr(ages, "Grace", 0)))
  io::print(toString(collections::getOr(ages, "Ada", 0)))
  RETURN 0
END FUNC
```

Look up every key of a map without a separate membership test:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  FOR EACH name IN collections::keys(ages)
    io::print(name & " is " & toString(collections::getOr(ages, name, 0)))
  NEXT
  RETURN 0
END FUNC
```

## See also

- `mfb man collections get`
- `mfb man collections hasKey`
- `mfb man collections keys`
- `mfb man collections set`
- `mfb man collections`
