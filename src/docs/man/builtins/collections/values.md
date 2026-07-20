# values

Return a map's values as a list.

## Synopsis

```
collections::values OF K, V(value AS Map OF K TO V) AS List OF V
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

`values` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `values` built-in.
[[src/builtins/collections.rs:is_native_member]]

## Description

`collections::values` builds a new `List OF V` holding the value of every entry
in `value`. It walks the map's lookup-entry table front to back, copying each
entry's value payload into a freshly allocated list block. The source map is not
mutated and its own storage is not aliased by the result — the returned list is
an independent, owned collection.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_values_builtin]]
[[src/target/shared/code/builder_collection_queries.rs:lower_map_projection]]

The result has exactly one item per map entry, so its length equals
`len(value)`. An empty map yields an empty list. Unlike the key projection, the
result may contain duplicates, because distinct keys may store equal values.
[[src/target/shared/code/builder_collection_queries.rs:lower_map_projection]]

**Ordering.** The projection walks the lookup-entry array directly, and that
array is maintained in insertion order; the hash bucket index is separate
derived metadata that does not reorder it. `collections::values` and
`collections::keys` are the same traversal over the same entries and differ only
in which payload field of each entry they copy, so the two results are
index-aligned: item `i` of `collections::values(m)` is the value of the entry
whose key is item `i` of `collections::keys(m)`. The language specification
describes map iteration order as implementation-defined but stable for a given
unchanged map, so treat insertion order as the current implementation's behavior
rather than a guarantee to rely on across versions.
[[src/target/shared/code/builder_collection_queries.rs:lower_map_projection]]

`collections::values` raises no trappable domain error, so an inline `TRAP` on a
`values` call has a dead handler. Building the result list does allocate, and an
allocation failure is not a trappable domain error in this language.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Map OF K TO V` | The map to project. Also accepted under the name `map`. Not mutated, and not aliased by the result. [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF V` | A new owned list of the map's values, one per entry, in the map's entry order; the empty list for an empty map. May contain duplicates. [[src/builtins/general.rs:resolve_values]] |

## Errors

No errors.

## Type checking

`collections::values` takes exactly one argument.
[[src/builtins/collections.rs:arity]]

It must be a `Map OF K TO V`; a `List` or any non-map value is a compile-time
type error. The result type is derived from the map's value type as
`List OF V`, so a `Map OF String TO Integer` yields a `List OF Integer`.
[[src/builtins/general.rs:resolve_values]]

## Examples

Get the values of a map:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET numbers AS List OF Integer = collections::values(ages)
  io::print(toString(len(numbers)))
  RETURN 0
END FUNC
```

Sum a map's values:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  io::print(toString(collections::sum(collections::values(ages))))
  RETURN 0
END FUNC
```

Iterate the values directly:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  FOR EACH age IN collections::values(ages)
    io::print(toString(age))
  NEXT
  RETURN 0
END FUNC
```

## See also

- `mfb man collections keys`
- `mfb man collections sum`
- `mfb man collections getOr`
- `mfb man collections mapValues`
- `mfb man collections`
