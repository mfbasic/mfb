# findLastIndex

Index of the last element at or before an end position that satisfies a predicate

## Synopsis

```
collections::findLastIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, endIndex AS Integer = -1) AS Integer
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

## Description

`collections::findLastIndex` scans `value` **backward**, beginning at the
element selected by `endIndex` and decreasing by one down to index `0`, calling
`predicate` with each element. It returns the zero-based index of the first
element (in that backward order) for which `predicate` returns `TRUE` — that is,
the last matching element at or before `endIndex`. The scan short-circuits at
that element: no lower index is examined. When the scan passes index `0` without
a match, the call raises `ErrNotFound` (`77050004`) rather than returning a
sentinel index. [[src/builtins/collections_package.mfb:__collections_findLastIndex]]

The third parameter is named `endIndex`. It is resolved in two steps, and the
order matters:

1. **Negative resolution.** A negative `endIndex` counts from the end of the
   list: the effective index becomes `len(value) + endIndex`. The default of
   `-1` therefore selects the last element, so the common call form scans the
   whole list from its end. A non-negative `endIndex` is used as written.
2. **Range check.** *After* resolution, the call raises `ErrIndexOutOfRange`
   (`77050001`) when the resolved index is less than `0` or greater than or
   equal to `len(value)`.

[[src/builtins/collections_package.mfb:__collections_findLastIndex]]

Because the range check runs on the resolved index, the upper bound is
`len(value) - 1`, not `len(value)`. This is deliberately asymmetric with
`collections::findIndex`, whose `start` may equal `len(value)` and whose
negative values are rejected instead of resolved.

One consequence is worth stating explicitly: on an **empty** list `len(value)`
is `0`, so every `endIndex` resolves outside `0 .. -1` and is rejected. The
default `-1` resolves to `-1`, which fails the range check. `findLastIndex` on
an empty list therefore raises `ErrIndexOutOfRange` (`77050001`), **not**
`ErrNotFound`. A caller that treats "no match" and "empty input" alike must
handle both codes.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findLastIndex` call to
the caller rather than being reported as a non-match. Note that a lambda passed
here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findLastIndex`. [[src/builtins/mod.rs:is_nonescaping_callback_arg]]

`findLastIndex` is a generic implemented in MFBASIC source; a call is rewritten
to the internal `__collections_findLastIndex` generic and instantiated for the
element type like any other generic function. [[src/builtins/collections.rs:FUNCTIONS]]
It does not mutate `value`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to scan. Not modified. An empty list always raises `ErrIndexOutOfRange`. [[src/builtins/collections_package.mfb:__collections_findLastIndex]] |
| `predicate` | `FUNC(T) AS Boolean` | Test applied to each element from the resolved end position downward; the scan stops at the first call returning `TRUE`. An error it raises propagates to the caller. [[src/builtins/collections_package.mfb:__collections_findLastIndex]] |
| `endIndex` | `Integer` | Zero-based index at which the backward scan begins. Optional, default `-1`. A negative value is resolved as `len(value) + endIndex`, so `-1` is the last element and `-len(value)` is the first; after resolution the index must satisfy `0 <= index < len(value)`. [[src/builtins/collections_package.mfb:__collections_findLastIndex]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The zero-based index of the last element at or before the resolved `endIndex` for which `predicate` returns `TRUE`. The result is always in `0 .. resolved endIndex`; there is no sentinel for "not found" — that case raises instead. [[src/builtins/collections_package.mfb:__collections_findLastIndex]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | After negative resolution, the effective end index is less than `0` or greater than or equal to `len(value)`. This includes every call on an empty list, and any `endIndex` more negative than `-len(value)`. [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |
| `77050004` | `ErrNotFound` | The backward scan reaches index `0` without `predicate` returning `TRUE`. [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |

## Type checking

`T` is inferred from the element type of `value` and may be any type;
`findLastIndex` imposes no comparability or orderability constraint on `T`,
because elements are never compared to one another — they are only passed to
`predicate`. The second argument must be a function value taking exactly one `T`
and returning `Boolean`, and `endIndex`, when supplied, must be an `Integer`.
[[src/builtins/collections_package.mfb:__collections_findLastIndex]]

## Examples

Find the last positive element:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findLastIndex([1, 2, 0, 3], isPos)))
  RETURN 0
END FUNC
```

Limit the backward scan with an explicit `endIndex`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET nums AS List OF Integer = [5, 0, 7, 9]
  io::print(toString(collections::findLastIndex(nums, isPos, 2)))
  RETURN 0
END FUNC
```

The parameter is named `endIndex`, so this is the named-argument spelling:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findLastIndex([5, 0, 7], isPos, endIndex := -2)))
  RETURN 0
END FUNC
```

An empty list raises `ErrIndexOutOfRange`, so a defensive caller traps both
codes:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC lastPositive(nums AS List OF Integer) AS Integer
  RETURN collections::findLastIndex(nums, isPos)

  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(lastPositive(empty)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections findIndex`
- `mfb man collections find`
- `mfb man collections any`
- `mfb man collections contains`
- `mfb man collections reduceRight`
