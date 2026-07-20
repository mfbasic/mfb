# collections

Sequence and map helper functions

## Synopsis

```
IMPORT collections
collections::get(value, indexOrKey)
collections::transform(value, f)
collections::filter(value, predicate)
collections::reduce(value, initial, f)
collections::sort(value)
collections::groupBy(value, keyFn, valFn)
```

## Description

The `collections` package provides package-qualified helpers for `List` and `Map`
values: element access and mutation (`get`, `set`, `append`, `prepend`, `insert`,
`removeAt`, `removeKey`), higher-order transforms (`transform`, `filter`,
`reduce`, `reduceRight`, `forEach`, `mapValues`), queries (`find`, `findIndex`,
`findLastIndex`, `contains`, `any`, `all`, `hasKey`, `keys`, `values`), reshaping
(`sort`, `sortBy`, `distinct`, `flatten`, `zip`, `chunks`, `window`, `partition`,
`groupBy`, `merge`), and numeric folding (`sum`). `collections` is a built-in
package: `IMPORT collections` needs no manifest dependency. [[src/builtins/collections.rs:FUNCTIONS]] [[src/builtins/collections.rs:NATIVE_MEMBERS]]

These helpers do not mutate their arguments. A function that changes a collection
returns a new value and leaves the original unchanged. List indexes are
zero-based, and access reads without copying the collection.

Element and key types follow the comparable/orderable rules: `sort` and `sortBy`
require an orderable element or key type, and `distinct` requires a comparable
element type. Map helpers operate on `Map OF K TO V` values, where the key type
`K` is the map's declared key type.

Predicates and other function arguments are passed as function values: a named
`FUNC`, a `LAMBDA`, or a general built-in predicate such as `isEven`,
`isPositive` or `isEmpty`. A built-in predicate resolves against the type
expected at that position — the element type of the list for a higher-order
call, or the declared type of a `FUNC(T) AS Boolean` binding — because a bare
name like `isPositive` is defined over `Integer`, `Float` and `Fixed` and nothing
in the reference alone chooses between them.

Some helpers introduce built-in result types: `zip` produces a `List OF Pair OF
A, B`, and `partition` produces a `Partition OF T` holding the matched and
unmatched elements. See `mfb man types pair` and `mfb man types partition`.

The List-only overloads of `find`, `mid`, and `replace` live here; their String
overloads live in `strings::`. [[src/builtins/collections.rs:resolve_call]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | raised by `get` (List overload), `set`, `insert`, `removeAt`, `mid`, `find`, `findIndex`, and `findLastIndex` when an index or start position is outside the valid range of the list [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |
| `77050002` | `ErrInvalidArgument` | raised by `chunks` when `chunkSize` is less than 1, and by `window` when `size` or `step` is less than 1 [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050004` | `ErrNotFound` | raised by `get` (Map overload) when the key is missing, and by `find`, `findIndex`, and `findLastIndex` when no element matches [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |
| `77050010` | `ErrOverflow` | raised by `sum` when Integer or Fixed summation exceeds the range of the element type [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
