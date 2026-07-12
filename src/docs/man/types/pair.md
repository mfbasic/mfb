# pair

The built-in Pair OF A, B two-value product

## Synopsis

```
Pair OF A, B
```

## Description

`Pair OF A, B` is a compiler-owned, always-in-scope generic record that holds two
values of possibly different types:

```
TYPE Pair OF A, B
  first  AS A
  second AS B
END TYPE
```

Unlike `MapEntry`, `Pair` places no comparability constraint on `A` or `B`. It is
a general two-value product, used by `collections::zip` to pair items from two
lists position-wise. `Pair` is an ordinary record — public, constructible with
`Pair[a, b]`, copyable when both `A` and `B` are copyable, and sendable across a
thread when both members are sendable — so it may be returned, stored in
collections, and passed between threads. `Pair OF A, B` is defaultable when both
`A` and `B` are defaultable. The name `Pair` is reserved: a user `TYPE` may not
redeclare it.

## Construction

Construct a `Pair` positionally; the type arguments are inferred from the
expected type, and the fields are read with member access:

```
LET p AS Pair OF Integer, String = Pair[1, "one"]
LET n AS Integer = p.first
LET s AS String = p.second
```

## Errors

No errors.

## Examples

Pair the elements of two lists:

```
IMPORT collections

LET pairs AS List OF Pair OF Integer, String = collections::zip([1, 2], ["a", "b"])
LET first AS Pair OF Integer, String = collections::get(pairs, 0)
```

## See also

- `mfb man types map`
- `mfb man types partition`
- `mfb man collections zip`
