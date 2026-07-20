# hasKey

Test whether a map contains an entry for a key.

## Synopsis

```
collections::hasKey OF K, V(value AS Map OF K TO V, key AS K) AS Boolean
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

`hasKey` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `hasKey` built-in.
[[src/builtins/collections.rs:is_native_member]]

## Description

`collections::hasKey` returns `TRUE` when `value` holds an entry whose key
matches `key`, and `FALSE` otherwise. The map is neither copied nor mutated, and
the matching value is never materialized — only the key is compared.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_has_key]]

This is a map-only member. There is no list or `String` form: to test list
membership use `collections::contains`, and to test for a substring use the
`strings::` package. [[src/builtins/collections.rs:expected_arguments]]

Key comparison is a comparison of the stored key payload. Fixed-width keys
compare their raw stored bits (one byte for `Boolean` and `Byte`, four for
`Scalar`, eight for `Integer`, `Float`, `Fixed`, and `Money`), and a `String`
key compares its length first and then its bytes. Because the comparison is
bitwise, a `Float` key of `NaN` never reports as present and `-0.0` does not
match a stored `0.0`.
[[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_matches_value_branch]]

For the key types `String`, `Integer`, `Float`, `Fixed`, `Byte`, and `Boolean`
the probe uses the map's hash bucket index; other key types use a linear scan of
the entry table. Both paths compare exactly the same key bytes and return the
same answer.
[[src/target/shared/code/builder_collection_query.rs:map_key_probe_eligible]]

`collections::hasKey` raises no trappable domain error, so an inline `TRAP` on a
`hasKey` call has a dead handler.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

Use `hasKey` to guard a `collections::get`, which *does* fail on a missing key.
When the goal is simply to obtain a value with a fallback,
`collections::getOr` does it in one call and avoids the second lookup.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Map OF K TO V` | The map to test. Also accepted under the name `map`. Not copied and not mutated. [[src/builtins/collections.rs:call_param_names]] |
| `key` | `K` | The key to look for. Must be exactly the map's key type. [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `value` has an entry for `key`; `FALSE` when it does not, including for every key when the map is empty. [[src/builtins/general.rs:resolve_has_key]] |

## Errors

No errors.

## Type checking

`collections::hasKey` takes exactly two arguments.
[[src/builtins/collections.rs:arity]]

The first must be a `Map OF K TO V`; a `List` or any non-map value is a
compile-time type error. The second must be exactly the map key type `K` — there
is no conversion, so probing a `Map OF String TO Integer` with an `Integer` does
not compile. The result is always `Boolean`, independently of `V`.
[[src/builtins/general.rs:resolve_has_key]]

## Examples

Test map membership:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  io::print(toString(collections::hasKey(ages, "Ada")))
  io::print(toString(collections::hasKey(ages, "Grace")))
  RETURN 0
END FUNC
```

Guard a lookup that would otherwise fail:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  IF collections::hasKey(ages, "Ada") THEN
    io::print(toString(collections::get(ages, "Ada")))
  END IF
  RETURN 0
END FUNC
```

Confirm that removing a key takes it out of the result:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET without AS Map OF String TO Integer = collections::removeKey(ages, "Ada")
  io::print(toString(collections::hasKey(without, "Ada")))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections get`
- `mfb man collections getOr`
- `mfb man collections removeKey`
- `mfb man collections keys`
- `mfb man collections contains`
- `mfb man collections`
