# partition

The built-in Partition OF T split result

## Synopsis

```
Partition OF T
```

## Description

`Partition OF T` is a compiler-owned, always-in-scope generic record returned by
`collections::partition`. It splits a list into the items a predicate matched and
the items it did not:

```
TYPE Partition OF T
  matched   AS List OF T
  unmatched AS List OF T
END TYPE
```

`matched` holds the items for which the predicate returned `TRUE` and `unmatched`
the rest, each in original order. `Partition` is an ordinary record — public,
constructible with `Partition[matched, unmatched]`, copyable when `T` is
copyable, and sendable across a thread when `T` is sendable. `Partition OF T` is
defaultable when `T` is defaultable, and its default holds two empty lists. The
name `Partition` is reserved: a user `TYPE` may not redeclare it.

## Construction

Construct a `Partition` positionally; the type argument is inferred from the
expected type, and the fields are read with member access:

```
LET p AS Partition OF Integer = Partition[[2, 4], [1, 3]]
LET evens AS List OF Integer = p.matched
LET odds AS List OF Integer = p.unmatched
```

## Errors

No errors.

## Examples

Split a list by a predicate in one pass:

```
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC classify() AS Integer
  LET result AS Partition OF Integer = collections::partition([1, -2, 3, -4], isPos)
  LET kept AS List OF Integer = result.matched
  LET rest AS List OF Integer = result.unmatched
  RETURN 0
END FUNC
```

## See also

- `mfb man types list`
- `mfb man types pair`
- `mfb man collections partition`
