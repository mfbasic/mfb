# removeKey

Return a copy of a map with the entry for one key removed.

## Synopsis

```
collections::removeKey OF K, V(value AS Map OF K TO V, key AS K) AS Map OF K TO V
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

`removeKey` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `removeKey` built-in.
[[src/builtins/collections.rs:is_native_member]]

## Description

`collections::removeKey` produces a **new** map containing every entry of
`value` except the one whose key matches `key`. It does not edit `value` in
place: the lowering scans the entry table to count the entries it will retain
and size their payloads, allocates a fresh map block, and copies the retained
entries into it. The original map is left untouched and remains usable.
[[src/target/shared/code/collection_mutate.rs:lower_collection_remove_key]]
[[src/target/shared/code/map_mutate.rs:lower_map_remove_key]]

Retained entries are copied in their existing order, so the surviving entries of
the result keep the relative order they had in `value`.
[[src/target/shared/code/map_mutate.rs:lower_map_remove_key]]

Removing a key that is not present is not an error. The scan simply retains
every entry, and the call returns a fresh map with the same contents as `value`.
Note that this is a new map rather than the same map object — a `removeKey` for
an absent key still allocates and copies, it does not return the argument
itself. The result therefore has `len(value)` entries when `key` was absent, or
`len(value) - 1` entries when it was present. Because a map holds at most one
entry per key, at most one entry is ever dropped.
[[src/target/shared/code/map_mutate.rs:lower_map_remove_key]]

Key comparison is a comparison of the stored key payload: fixed-width keys
compare their raw stored bits and a `String` key compares length and then bytes.
Since the comparison is bitwise, a `Float` key of `NaN` matches no entry, so
such a call always returns an unchanged copy.
[[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_matches_value_branch]]

`collections::removeKey` raises no trappable domain error — neither a missing
key nor an empty map fails — so an inline `TRAP` on a `removeKey` call has a
dead handler. Building the result map does allocate, and an allocation failure
is not a trappable domain error in this language.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Map OF K TO V` | The map to copy from. Also accepted under the name `map`. Not mutated. [[src/builtins/collections.rs:call_param_names]] |
| `key` | `K` | The key whose entry is dropped from the copy. Must be exactly the map's key type. A key that is not present is accepted and drops nothing. [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Map OF K TO V` | A new owned map of the same type, holding every entry of `value` except the one for `key`. Has one fewer entry than `value` when `key` was present, and the same entries as `value` when it was not. [[src/builtins/general.rs:resolve_remove_key]] |

## Errors

No errors.

## Type checking

`collections::removeKey` takes exactly two arguments.
[[src/builtins/collections.rs:arity]]

The first must be a `Map OF K TO V`; a `List` or any non-map value is a
compile-time type error, and there is no list counterpart here — use
`collections::removeAt` to drop a list item by index. The second must be exactly
the map key type `K`. The result has the same map type as the first argument.
[[src/builtins/general.rs:resolve_remove_key]]

## Examples

Remove a key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  LET smaller AS Map OF String TO Integer = collections::removeKey(ages, "Ada")
  io::print(toString(len(smaller)))
  io::print(toString(collections::hasKey(smaller, "Ada")))
  RETURN 0
END FUNC
```

The original map is unchanged:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET smaller AS Map OF String TO Integer = collections::removeKey(ages, "Ada")
  io::print(toString(collections::hasKey(ages, "Ada")))
  RETURN 0
END FUNC
```

Removing an absent key is harmless:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET same AS Map OF String TO Integer = collections::removeKey(ages, "Grace")
  io::print(toString(len(same)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections hasKey`
- `mfb man collections set`
- `mfb man collections keys`
- `mfb man collections removeAt`
- `mfb man collections`
