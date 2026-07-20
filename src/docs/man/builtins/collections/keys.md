# keys

Return a map's keys as a list.

## Synopsis

```
collections::keys OF K, V(value AS Map OF K TO V) AS List OF K
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

`keys` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `keys` built-in.
[[src/builtins/collections.rs:is_native_member]]

## Description

`collections::keys` builds a new `List OF K` holding the key of every entry in
`value`. It walks the map's lookup-entry table front to back, copying each
entry's key payload into a freshly allocated list block. The source map is not
mutated and its own storage is not aliased by the result — the returned list is
an independent, owned collection.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_keys]]
[[src/target/shared/code/builder_collection_queries.rs:lower_map_projection]]

The result has exactly one item per map entry, so its length equals
`len(value)`. An empty map yields an empty list. Each key appears exactly once,
because a map holds at most one entry per key.
[[src/target/shared/code/builder_collection_queries.rs:lower_map_projection]]

**Ordering.** The projection walks the lookup-entry array directly, and that
array is maintained in insertion order; the hash bucket index is separate
derived metadata that does not reorder it. `collections::keys` and
`collections::values` walk the same array over the same entries and differ only
in which payload field of each entry they copy, so the two results are
index-aligned: item `i` of `collections::keys(m)` is the key of the entry whose
value is item `i` of `collections::values(m)`. The language specification
describes map iteration order as implementation-defined but stable for a given
unchanged map, so treat insertion order as the current implementation's behavior
rather than a guarantee to rely on across versions.
[[src/target/shared/code/builder_collection_queries.rs:lower_map_projection]]

`collections::keys` raises no trappable domain error, so an inline `TRAP` on a
`keys` call has a dead handler. Building the result list does allocate, and an
allocation failure is not a trappable domain error in this language.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Map OF K TO V` | The map to project. Also accepted under the name `map`. Not mutated, and not aliased by the result. [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF K` | A new owned list of the map's keys, one per entry, in the map's entry order; the empty list for an empty map. [[src/builtins/general.rs:resolve_keys]] |

## Errors

No errors.

## Type checking

`collections::keys` takes exactly one argument.
[[src/builtins/collections.rs:arity]]

It must be a `Map OF K TO V`; a `List` or any non-map value is a compile-time
type error. The result type is derived from the map's key type as
`List OF K`, so a `Map OF String TO Integer` yields a `List OF String`.
[[src/builtins/general.rs:resolve_keys]]

## Examples

Get the keys of a map:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET names AS List OF String = collections::keys(ages)
  io::print(toString(len(names)))
  RETURN 0
END FUNC
```

Iterate a map by key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  FOR EACH name IN collections::keys(ages)
    io::print(name & " is " & toString(collections::getOr(ages, name, 0)))
  NEXT
  RETURN 0
END FUNC
```

The keys and values projections line up index for index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  LET names AS List OF String = collections::keys(ages)
  LET numbers AS List OF Integer = collections::values(ages)
  io::print(collections::get(names, 0) & "=" & toString(collections::get(numbers, 0)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections values`
- `mfb man collections hasKey`
- `mfb man collections getOr`
- `mfb man collections removeKey`
- `mfb man collections`
