# findIndex

Index of the first element at or after a start position that satisfies a predicate

## Synopsis

```
collections::findIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, start AS Integer = 0) AS Integer
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

`collections::findIndex` scans `value` **forward**, beginning at index `start`
and advancing by one, calling `predicate` with each element. It returns the
zero-based index of the first element for which `predicate` returns `TRUE`. The
scan short-circuits at that element: no later element is examined. When the scan
reaches the end of the list without a match, the call raises `ErrNotFound`
(`77050004`) rather than returning a sentinel index. [[src/builtins/collections_package.mfb:__collections_findIndex]]

`start` defaults to `0`, so the common call form scans the whole list. It is
validated **before** any element is read: the call raises `ErrIndexOutOfRange`
(`77050001`) when `start < 0` or `start > len(value)`. Two consequences are
worth stating precisely:

- `start` equal to `len(value)` is **legal**. It selects an empty scan, so the
  call raises `ErrNotFound`, not `ErrIndexOutOfRange`. `start` strictly greater
  than `len(value)` is the out-of-range case.
- A negative `start` is **not** interpreted as an offset from the end of the
  list. It is simply out of range and raises `ErrIndexOutOfRange`. This is
  deliberately asymmetric with `collections::findLastIndex`, whose `endIndex`
  parameter *does* resolve negative values from the end.

On an empty list every legal `start` is `0`, which is `len(value)`, so
`findIndex` on an empty list raises `ErrNotFound`.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findIndex` call to the
caller rather than being reported as a non-match. Note that a lambda passed here
may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findIndex`. [[src/builtins/mod.rs:is_nonescaping_callback_arg]]

`findIndex` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_findIndex` generic and instantiated for the element
type like any other generic function. [[src/builtins/collections.rs:FUNCTIONS]] It
does not mutate `value`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to scan. Not modified. [[src/builtins/collections_package.mfb:__collections_findIndex]] |
| `predicate` | `FUNC(T) AS Boolean` | Test applied to each element from `start` upward; the scan stops at the first call returning `TRUE`. An error it raises propagates to the caller. [[src/builtins/collections_package.mfb:__collections_findIndex]] |
| `start` | `Integer` | Zero-based index at which the forward scan begins. Optional, default `0`. Must satisfy `0 <= start <= len(value)`; a negative value is out of range, not an offset from the end. [[src/builtins/collections_package.mfb:__collections_findIndex]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The zero-based index of the first element at or after `start` for which `predicate` returns `TRUE`. The result is always in `start .. len(value) - 1`; there is no sentinel for "not found" — that case raises instead. [[src/builtins/collections_package.mfb:__collections_findIndex]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `start` is less than `0` or greater than `len(value)`, checked before the scan begins. [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |
| `77050004` | `ErrNotFound` | The scan reaches the end of `value` without `predicate` returning `TRUE`, including the empty scan produced by `start = len(value)`. [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |

## Type checking

`T` is inferred from the element type of `value` and may be any type;
`findIndex` imposes no comparability or orderability constraint on `T`, because
elements are never compared to one another — they are only passed to
`predicate`. The second argument must be a function value taking exactly one `T`
and returning `Boolean`, and `start`, when supplied, must be an `Integer`.
[[src/builtins/collections_package.mfb:__collections_findIndex]]

## Examples

Find the first positive element:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findIndex([-1, 0, 3, 4], isPos)))
  RETURN 0
END FUNC
```

Resume the scan past an earlier match by passing `start`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET nums AS List OF Integer = [5, -1, 7, -2]
  LET first AS Integer = collections::findIndex(nums, isPos)
  io::print(toString(collections::findIndex(nums, isPos, first + 1)))
  RETURN 0
END FUNC
```

Handle the no-match case with a function-level `TRAP`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC firstPositive(nums AS List OF Integer) AS Integer
  RETURN collections::findIndex(nums, isPos)

  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

FUNC main AS Integer
  io::print(toString(firstPositive([-3, -2])))
  RETURN 0
END FUNC
```

The third parameter is named `start`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findIndex([5, -1, 7], isPos, start := 1)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections findLastIndex`
- `mfb man collections find`
- `mfb man collections any`
- `mfb man collections contains`
- `mfb man collections filter`
