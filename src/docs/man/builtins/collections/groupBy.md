# groupBy

Group the items of a list into a map of lists keyed by a projection

## Synopsis

```
collections::groupBy OF T, K, V(value AS List OF T, keyFn AS FUNC(T) AS K, valFn AS FUNC(T) AS V) AS Map OF K TO List OF V
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

`collections::groupBy` builds a `Map OF K TO List OF V` from `value`. It first
projects the whole list twice: `keyFn` over every item to produce the group key,
and `valFn` over every item to produce the value stored in that group's bucket.
Both projections run over the entire list up front, via `collections::transform`,
before any bucket is written. It then walks the two projected lists in parallel
in list order, appending each projected value to the bucket for its key, creating
the bucket on first use. [[src/builtins/collections_package.mfb:__collections_groupBy]]

Because the walk proceeds in list order and each value is appended to the end of
its bucket, the items inside a bucket appear in the same relative order they had
in `value`. `groupBy` never merges, reorders, or deduplicates within a bucket:
two items that produce equal keys *and* equal values both appear.

`groupBy` takes three arguments. There is no single-argument-projection form that
groups items by a key and stores the original items — pass an identity `FUNC` as
`valFn` to get that behavior. Calling it with two arguments is a compile-time
error, because the compiler cannot infer the template argument `V` (it appears
only in the return type). [[src/builtins/collections_package.mfb:__collections_groupBy]]

`value` is not modified; the result is a newly built map. The key type `K` must
be a usable map key type, since the result is a `Map OF K TO List OF V`.

`keyFn` and `valFn` are ordinary MFBASIC function values and are called with
ordinary calls. If either callback fails, its error propagates out of `groupBy`
to the caller and can be caught by the caller's `TRAP` block; the partially built
map is discarded. `groupBy` itself raises no error of its own.
[[src/builtins/collections_package.mfb:__collections_groupBy]]

Either callback may be a named `FUNC` or a `LAMBDA` expression, since both
produce a function value of the required type.
[[src/ast/expr.rs:parse_lambda]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to group. May be empty, in which case the result is an empty map. Not modified. [[src/builtins/collections_package.mfb:__collections_groupBy]] |
| `keyFn` | `FUNC(T) AS K` | Projection producing the group key for an item. Applied to every item of `value`, including items whose key already exists. [[src/builtins/collections_package.mfb:__collections_groupBy]] |
| `valFn` | `FUNC(T) AS V` | Projection producing the value stored in the group's bucket for an item. Applied to every item of `value`. [[src/builtins/collections_package.mfb:__collections_groupBy]] |

## Return value

| Type | Description |
| --- | --- |
| `Map OF K TO List OF V` | A new map holding one entry per distinct key produced by `keyFn`. Each entry's value is the list of `valFn` results for the items that produced that key, in their original list order. Empty when `value` is empty. [[src/builtins/collections_package.mfb:__collections_groupBy]] |

## Errors

No errors.

## Type checking

`groupBy` is generic over three template parameters: `T`, the element type of
`value`; `K`, the key type returned by `keyFn`; and `V`, the value type returned
by `valFn`. All three are inferred from the argument types, so every one of them
must be determined by an argument — `V` cannot be supplied from the annotation on
the binding that receives the result. `K` must be a valid map key type.
[[src/builtins/collections_package.mfb:__collections_groupBy]]

## Examples

Group numbers by parity, keeping the numbers themselves:

```
IMPORT io
IMPORT collections

FUNC parity(n AS Integer) AS Integer
  RETURN n MOD 2
END FUNC

FUNC identity(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET groups AS Map OF Integer TO List OF Integer = collections::groupBy(nums, parity, identity)
  io::print(toString(len(collections::get(groups, 0))))
  RETURN 0
END FUNC
```

The same grouping written with lambdas and named arguments:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET groups AS Map OF Integer TO List OF Integer = collections::groupBy(value := nums, keyFn := LAMBDA(n AS Integer) -> n MOD 2, valFn := LAMBDA(n AS Integer) -> n)
  io::print(toString(len(collections::keys(groups))))
  RETURN 0
END FUNC
```

A failing projection propagates its error to the caller's `TRAP`:

```
IMPORT io
IMPORT collections

FUNC strictKey(n AS Integer) AS Integer
  IF n < 0 THEN
    FAIL error(77050002, "negative item")
  END IF
  RETURN n MOD 2
END FUNC

FUNC identity(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main AS Integer
  LET groups AS Map OF Integer TO List OF Integer = collections::groupBy([1, -2, 3], strictKey, identity)
  io::print(toString(len(collections::keys(groups))))
  RETURN 0
  TRAP(err)
    io::print("failed: " & toString(err.code))
    RETURN 1
  END TRAP
END FUNC
```

## See also

- `mfb man collections mapValues`
- `mfb man collections partition`
- `mfb man collections distinct`
- `mfb man collections transform`
- `mfb man collections merge`
