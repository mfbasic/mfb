# partition

Split a list into the elements that satisfy a predicate and those that do not

## Synopsis

```
collections::partition OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Partition OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

The `Partition OF T` result type needs no import of its own: it is a
compiler-owned generic record template in the always-in-scope builtin prelude.
[[src/ast/manifest.rs:builtin_prelude_file]]

## Description

`collections::partition` walks `value` once, from index `0` upward, calling
`predicate` with each element. Each element is appended to the `matched` list
when `predicate` returns `TRUE` and to the `unmatched` list otherwise, and the
two lists are returned together in a single `Partition OF T` record.
[[src/builtins/collections_package.mfb:__collections_partition]]

Unlike `collections::any` and `collections::all`, `partition` does **not**
short-circuit: `predicate` is called exactly once for every element of `value`,
in index order, because every element must be classified.

Order is preserved within each side. Elements keep their original relative order
inside `matched` and inside `unmatched`; concatenating the two does not in
general reconstruct `value`, but each side on its own is a subsequence of it.
Every element lands on exactly one side, so `len(result.matched) +
len(result.unmatched)` always equals `len(value)`. An empty input yields a
`Partition` whose two lists are both empty.

The result type `Partition OF T` is an ordinary generic record with two fields,
`matched` and `unmatched`, both of type `List OF T`. It is constructed and
field-accessed like any other record — write `result.matched` — and it is
declared in the compiler-owned prelude injected into every project, so it is in
scope without an import. [[src/ast/manifest.rs:builtin_prelude_file]]

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` is **not** absorbed by `partition`: it propagates out of the
`collections::partition` call to the caller, abandoning the partially built
result. `partition` itself defines no error of its own. Note that a lambda
passed here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `partition`. [[src/builtins/mod.rs:is_nonescaping_callback_arg]]

`partition` does not mutate `value`; it builds two new lists. It allocates while
doing so, but allocation failure is not a trappable domain error, and the
`append` it uses is classified infallible for exactly that reason.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`partition` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_partition` generic and instantiated for the element
type like any other generic function. [[src/builtins/collections.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to split, visited in index order from `0`. An empty list is accepted and yields two empty lists. Not modified. [[src/builtins/collections_package.mfb:__collections_partition]] |
| `predicate` | `FUNC(T) AS Boolean` | Classifier applied to every element exactly once. `TRUE` sends the element to `matched`, `FALSE` to `unmatched`. An error it raises propagates to the caller. [[src/builtins/collections_package.mfb:__collections_partition]] |

## Return value

| Type | Description |
| --- | --- |
| `Partition OF T` | A record with fields `matched AS List OF T` (the elements for which `predicate` returned `TRUE`) and `unmatched AS List OF T` (the rest), each in original relative order. Their lengths always sum to `len(value)`. [[src/builtins/collections_package.mfb:__collections_partition]] [[src/ast/manifest.rs:builtin_prelude_file]] |

## Errors

No errors.

## Type checking

`T` is inferred from the element type of `value` and may be any type;
`partition` imposes no comparability or orderability constraint on `T`, because
elements are never compared to one another — they are only passed to
`predicate`. The second argument must be a function value taking exactly one `T`
and returning `Boolean`. The result binding, when annotated, is written
`Partition OF T` with the same `T` as the input's element type.
[[src/builtins/collections_package.mfb:__collections_partition]]

## Examples

Split integers into positives and the rest:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET result AS Partition OF Integer = collections::partition([-1, 2, -3, 4], isPos)
  io::print(toString(len(result.matched)))
  io::print(toString(len(result.unmatched)))
  RETURN 0
END FUNC
```

Each side keeps its original order:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET result AS Partition OF Integer = collections::partition([3, -1, 5, -2], isPos)
  io::print(toString(collections::get(result.matched, 0)))
  io::print(toString(collections::get(result.matched, 1)))
  io::print(toString(collections::get(result.unmatched, 0)))
  RETURN 0
END FUNC
```

Named arguments bind by the declared parameter names `value` and `predicate`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET result AS Partition OF Integer = collections::partition(value := [1, -1], predicate := isPos)
  io::print(toString(len(result.matched)))
  RETURN 0
END FUNC
```

## See also

- `mfb man types partition`
- `mfb man collections filter`
- `mfb man collections groupBy`
- `mfb man collections any`
- `mfb man collections all`
