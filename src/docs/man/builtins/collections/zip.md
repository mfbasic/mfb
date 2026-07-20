# zip

Pair items from two lists position-wise

## Synopsis

```
collections::zip OF A, B(a AS List OF A, b AS List OF B) AS List OF Pair OF A, B
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

`collections::zip` combines two lists element by element. It first computes the
pairing length `n` as the smaller of `len(a)` and `len(b)`, then for each index
`i` from `0` to `n - 1` builds a `Pair OF A, B` whose `first` field is the item
of `a` at `i` and whose `second` field is the item of `b` at `i`, appending each
pair to the result. [[src/builtins/collections_package.mfb:__collections_zip]]

Pairing therefore stops at the shorter input: the result length is exactly
`min(len(a), len(b))`, and the trailing items of the longer list are dropped
without notice. There is no padding, no filler value, and no error — zipping a
3-item list with a 1-item list simply yields one pair. When either list is empty,
the result is empty. [[src/builtins/collections_package.mfb:__collections_zip]]

Positional correspondence is preserved: the pair at index `i` of the result
always holds the items that were at index `i` in both inputs, so the result reads
in the same order as the inputs.

`Pair OF A, B` is a compiler-owned prelude record template with the two fields
`first` and `second`. It is always in scope and needs no import, so a program can
name the result type `List OF Pair OF A, B` and read `p.first` / `p.second`
directly. [[src/ast/manifest.rs:builtin_prelude_file]]

Neither `a` nor `b` is modified; the result is a newly built list. `zip` invokes
no user callback and raises no error.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `List OF A` | The list supplying the `first` field of each pair. May be empty. Not modified. [[src/builtins/collections_package.mfb:__collections_zip]] |
| `b` | `List OF B` | The list supplying the `second` field of each pair. May be empty, and need not be the same length as `a`. Not modified. [[src/builtins/collections_package.mfb:__collections_zip]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Pair OF A, B` | A new list of `min(len(a), len(b))` pairs; the pair at index `i` holds `a`'s item at `i` as `first` and `b`'s item at `i` as `second`. Empty when either input is empty. [[src/builtins/collections_package.mfb:__collections_zip]] |

## Errors

No errors.

## Type checking

`zip` is generic over `A` and `B`, the element types of the two lists. They are
inferred independently from the two arguments and need not be the same type. The
result type is the prelude record template `Pair` instantiated as `Pair OF A, B`.
[[src/builtins/collections_package.mfb:__collections_zip]] [[src/ast/manifest.rs:builtin_prelude_file]]

## Examples

Pair numbers with labels and read both fields:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET pairs AS List OF Pair OF Integer, String = collections::zip([1, 2], ["a", "b"])
  LET p AS Pair OF Integer, String = collections::get(pairs, 0)
  io::print(toString(p.first) & p.second)
  RETURN 0
END FUNC
```

The shorter list decides the result length:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET short AS List OF Pair OF Integer, String = collections::zip(a := [1, 2, 3, 4], b := ["x"])
  io::print(toString(len(short)))
  RETURN 0
END FUNC
```

Walk a zipped list:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET pairs AS List OF Pair OF String, Integer = collections::zip(["a", "b"], [10, 20])
  FOR EACH p IN pairs
    io::print(p.first & "=" & toString(p.second))
  NEXT
  RETURN 0
END FUNC
```

## See also

- `mfb man types pair`
- `mfb man collections flatten`
- `mfb man collections chunks`
- `mfb man collections transform`
- `mfb man collections partition`
