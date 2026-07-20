# mapValues

Transform every value of a map, leaving the keys unchanged

## Synopsis

```
collections::mapValues OF K, V, U(value AS Map OF K TO V, f AS FUNC(V) AS U) AS Map OF K TO U
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

`collections::mapValues` builds a new `Map OF K TO U` by iterating `value` with
`FOR EACH` and, for each entry, storing the original `e.key` together with the
transformed value `f(e.value)`. The keys are copied through untouched, so the
result has exactly the same key set as `value` and the same number of entries.
Only the value type changes, from `V` to `U`. [[src/builtins/collections_package.mfb:__collections_mapValues]]

`f` is applied exactly once per entry in `value`. Because entries are written in
the order `FOR EACH` yields them, the result is built by inserting keys in the
source map's traversal order. Map traversal order is implementation-defined but
stable for a given unchanged map value during one program run, so no ordering
guarantee beyond that should be relied on; see `mfb man types map`.
[[src/builtins/collections_package.mfb:__collections_mapValues]]

`value` is not modified — the source map is read, and a separate result map is
constructed and returned. When `value` is empty, `f` is never called and the
result is an empty map.

`f` is an ordinary MFBASIC function value invoked with an ordinary call. If `f`
fails on some entry, its error propagates out of `mapValues` to the caller and
can be caught by the caller's `TRAP` block; the partially built result map is
discarded. `mapValues` itself raises no error of its own.

`f` may be a named `FUNC` or a `LAMBDA` expression, since both produce a function
value of the required type. [[src/ast/expr.rs:parse_lambda]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Map OF K TO V` | The source map. May be empty. Not modified. [[src/builtins/collections_package.mfb:__collections_mapValues]] |
| `f` | `FUNC(V) AS U` | Transform applied to each entry's value. Receives only the value; the entry's key is not passed to it. Called once per entry. [[src/builtins/collections_package.mfb:__collections_mapValues]] |

## Return value

| Type | Description |
| --- | --- |
| `Map OF K TO U` | A new map with the same keys as `value`, each mapped to `f` applied to that key's original value. Empty when `value` is empty. [[src/builtins/collections_package.mfb:__collections_mapValues]] |

## Errors

No errors.

## Type checking

`mapValues` is generic over `K` and `V`, the key and value types of the source
map, and `U`, the value type `f` returns. All three are inferred from the
argument types. `K` is carried straight through to the result type, so it must
remain a valid map key type; `U` may be any type, including `V` itself.
[[src/builtins/collections_package.mfb:__collections_mapValues]]

## Examples

Double every value in a map:

```
IMPORT io
IMPORT collections

FUNC double(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC main AS Integer
  LET scores AS Map OF String TO Integer = Map OF String TO Integer { "a" := 3, "b" := 4 }
  LET doubled AS Map OF String TO Integer = collections::mapValues(scores, double)
  io::print(toString(collections::get(doubled, "a")))
  RETURN 0
END FUNC
```

Change the value type, using a lambda and named arguments:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET scores AS Map OF String TO Integer = Map OF String TO Integer { "a" := 3 }
  LET labels AS Map OF String TO String = collections::mapValues(value := scores, f := LAMBDA(n AS Integer) -> toString(n))
  io::print(collections::get(labels, "a"))
  RETURN 0
END FUNC
```

The source map is left unchanged:

```
IMPORT io
IMPORT collections

FUNC double(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC main AS Integer
  LET scores AS Map OF String TO Integer = Map OF String TO Integer { "a" := 3 }
  LET doubled AS Map OF String TO Integer = collections::mapValues(scores, double)
  io::print(toString(collections::get(scores, "a")))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections transform`
- `mfb man collections merge`
- `mfb man collections groupBy`
- `mfb man collections keys`
- `mfb man collections values`
