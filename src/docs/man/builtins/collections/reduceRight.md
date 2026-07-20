# reduceRight

Fold a list into a single value, walking from the last item to the first

## Synopsis

```
collections::reduceRight OF T, U(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U
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

`collections::reduceRight` folds `value` into a single accumulated result. The
accumulator starts at `initial`. The function walks the list from the last index
down to index 0, and at each step replaces the accumulator with
`f(accumulator, item)`. When the walk finishes, the accumulator is returned.
[[src/builtins/collections_package.mfb:__collections_reduceRight]]

The accumulator is the **first** argument of `f` and the list item is the second
— the same argument order `collections::reduce` uses. Only the traversal
direction differs between the two: `reduce` moves from the first item to the
last, `reduceRight` from the last to the first. `f` is therefore declared as
`FUNC(U, T) AS U`, not `FUNC(T, U) AS U`.
[[src/builtins/collections_package.mfb:__collections_reduceRight]]

For a three-item list `[x, y, z]`, the result is
`f(f(f(initial, z), y), x)`. Direction matters whenever `f` is not associative
and commutative: folding `[1, 2, 3]` from the right with subtraction and an
initial accumulator of `0` yields `((0 - 3) - 2) - 1`, or `-6`.

`f` is called exactly once per item, so an empty `value` calls `f` not at all and
returns `initial` unchanged. `value` is not modified.

The accumulator type `U` need not match the element type `T`; `reduceRight` can
fold a list into a value of an entirely different type, such as building a
`String` from a `List OF Integer`.

`f` is an ordinary MFBASIC function value invoked with an ordinary call. If it
fails at any step, its error propagates out of `reduceRight` to the caller and
can be caught by the caller's `TRAP` block; the partially accumulated value is
discarded. `reduceRight` itself raises no error of its own.

`f` may be a named `FUNC` or a `LAMBDA` expression, since both produce a function
value of the required type. [[src/ast/expr.rs:parse_lambda]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to fold. Traversed from its last item to its first. May be empty. Not modified. [[src/builtins/collections_package.mfb:__collections_reduceRight]] |
| `initial` | `U` | The starting accumulator, and the result when `value` is empty. Determines the accumulator type `U`. [[src/builtins/collections_package.mfb:__collections_reduceRight]] |
| `f` | `FUNC(U, T) AS U` | The folding step. Receives the current accumulator first and the list item second, and returns the next accumulator. Called once per item. [[src/builtins/collections_package.mfb:__collections_reduceRight]] |

## Return value

| Type | Description |
| --- | --- |
| `U` | The accumulator after every item has been folded, right to left. Exactly `initial` when `value` is empty. [[src/builtins/collections_package.mfb:__collections_reduceRight]] |

## Errors

No errors.

## Type checking

`reduceRight` is generic over `T`, the element type of `value`, and `U`, the
accumulator type. `T` is inferred from `value` and `U` from `initial`; `f` must
accept them in the order `(U, T)` and return `U`, so a step function written with
the item first will not match. The two type parameters are independent — `U` may
be any type, including one unrelated to `T`.
[[src/builtins/collections_package.mfb:__collections_reduceRight]]

## Examples

Subtract each item from an accumulator, right to left:

```
IMPORT io
IMPORT collections

FUNC subtract(acc AS Integer, n AS Integer) AS Integer
  RETURN acc - n
END FUNC

FUNC main AS Integer
  LET total AS Integer = collections::reduceRight([1, 2, 3], 0, subtract)
  io::print(toString(total))
  RETURN 0
END FUNC
```

Fold into a different type — build a reversed `String` from a `List OF String`:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET words AS List OF String = ["a", "b", "c"]
  LET joined AS String = collections::reduceRight(value := words, initial := "", f := LAMBDA(acc AS String, w AS String) -> acc & w)
  io::print(joined)
  RETURN 0
END FUNC
```

An empty list returns `initial` untouched:

```
IMPORT io
IMPORT collections

FUNC subtract(acc AS Integer, n AS Integer) AS Integer
  RETURN acc - n
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::reduceRight(empty, 42, subtract)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections reduce`
- `mfb man collections sum`
- `mfb man collections transform`
- `mfb man collections filter`
- `mfb man collections forEach`
