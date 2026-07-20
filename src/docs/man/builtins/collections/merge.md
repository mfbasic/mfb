# merge

Combine two maps into one, choosing which side wins on a key collision

## Synopsis

```
collections::merge OF K, V(a AS Map OF K TO V, b AS Map OF K TO V, preferB AS Boolean) AS Map OF K TO V
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:FUNCTIONS]]

## Description

`collections::merge` starts the result as a copy of `a`, then iterates `b` with
`FOR EACH` and considers each of its entries in turn. An entry of `b` is written
into the result only when `preferB` is `TRUE`, or when the entry's key is not
already present in the result. Every other entry of `b` is skipped, leaving the
value that came from `a` in place. [[src/builtins/collections_package.mfb:__collections_merge]]

The result therefore always contains the union of the two key sets: every key of
`a` and every key of `b` appears exactly once. `preferB` decides only what
happens on a collision — a key present in both maps:

- `preferB = TRUE` — `b`'s value overwrites `a`'s value for that key.
- `preferB = FALSE` — `a`'s value is kept and `b`'s value is discarded.

`preferB` is a required `Boolean` parameter. It has no default, so all three
arguments must be supplied; there is no two-argument form of `merge`.
[[src/builtins/collections_package.mfb:__collections_merge]]

Neither `a` nor `b` is modified. The result is a distinct map value, so writing
to it afterwards does not disturb either input.

Because the result is seeded from `a` and then extended by iterating `b`, keys of
`a` are inserted first and keys unique to `b` are inserted afterwards, each side
in its own traversal order. Map traversal order is implementation-defined but
stable for a given unchanged map value during one program run, so no ordering
guarantee beyond that should be relied on; see `mfb man types map`. Note that
overwriting an existing key when `preferB` is `TRUE` replaces its value and does
not move the key to the end.

`merge` invokes no user callback and raises no error.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Map OF K TO V` | The base map. The result begins as a copy of it, so its entries survive unless a colliding entry of `b` displaces them. May be empty. Not modified. [[src/builtins/collections_package.mfb:__collections_merge]] |
| `b` | `Map OF K TO V` | The map merged on top. Must have the same key and value types as `a`. May be empty. Not modified. [[src/builtins/collections_package.mfb:__collections_merge]] |
| `preferB` | `Boolean` | Which side wins on a key present in both maps: `TRUE` takes `b`'s value, `FALSE` keeps `a`'s. Required — there is no default. [[src/builtins/collections_package.mfb:__collections_merge]] |

## Return value

| Type | Description |
| --- | --- |
| `Map OF K TO V` | A new map holding every key of `a` and of `b`. Colliding keys take `b`'s value when `preferB` is `TRUE` and `a`'s value otherwise. Equal to a copy of the other map when either input is empty. [[src/builtins/collections_package.mfb:__collections_merge]] |

## Errors

No errors.

## Type checking

`merge` is generic over `K` and `V`, the key and value types of the maps. Both
`a` and `b` bind the same `K` and the same `V`, so the two maps must have
identical key and value types; merging maps with different value types is a
compile-time error. The result is a `Map OF K TO V` with those same types.
[[src/builtins/collections_package.mfb:__collections_merge]]

## Examples

Let the second map win on a collision:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET a AS Map OF String TO Integer = Map OF String TO Integer { "x" := 1 }
  LET b AS Map OF String TO Integer = Map OF String TO Integer { "x" := 2, "y" := 9 }
  LET merged AS Map OF String TO Integer = collections::merge(a, b, TRUE)
  io::print(toString(collections::get(merged, "x")))
  RETURN 0
END FUNC
```

Keep the first map's value instead, while still gaining `b`'s new keys:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET defaults AS Map OF String TO Integer = Map OF String TO Integer { "retries" := 3 }
  LET overrides AS Map OF String TO Integer = Map OF String TO Integer { "retries" := 9, "timeout" := 30 }
  LET settings AS Map OF String TO Integer = collections::merge(a := defaults, b := overrides, preferB := FALSE)
  io::print(toString(collections::get(settings, "retries")) & " " & toString(collections::get(settings, "timeout")))
  RETURN 0
END FUNC
```

Both inputs are left unchanged:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET a AS Map OF String TO Integer = Map OF String TO Integer { "x" := 1 }
  LET b AS Map OF String TO Integer = Map OF String TO Integer { "x" := 2 }
  LET merged AS Map OF String TO Integer = collections::merge(a, b, TRUE)
  io::print(toString(collections::get(a, "x")))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections mapValues`
- `mfb man collections set`
- `mfb man collections hasKey`
- `mfb man collections keys`
- `mfb man collections groupBy`
