# take

Return a new list holding the first `count` elements of a list

## Synopsis

```
collections::take OF T(value AS List OF T, count AS Integer) AS List OF T
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

`collections::take` returns a new list containing the leading `count` elements
of `value`, in their original order. It is a generic function written in MFBASIC
source: the call is rewritten to the internal `__collections_take` generic and
instantiated for the element type `T` during monomorphization.
[[src/builtins/collections.rs:internal_name]]
[[src/builtins/collections_package.mfb:__collections_take]]

`take(value, count)` is defined as the half-open range `[0, count)` of `value`,
delegated to the internal slice helper. That helper is lowered natively as a
bulk range copy, and the native lowering is what defines the boundary behavior:
the range start is clamped into `[0, len]` and the range stop into
`[start, len]`. [[src/builtins/collections_package.mfb:__collections_take]]
[[src/target/shared/code/builder_collection_queries.rs:try_inline_slice_op]]
[[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]]

That clamping makes `take` **total** — every `Integer` value of `count` is
accepted and no index is ever rejected:

- `count` of 0 or any negative value clamps the stop back to the start, so the
  result is the empty list.
- `count` greater than or equal to the length of `value` clamps the stop to the
  length, so the whole list is returned.
- Otherwise the result holds exactly `count` elements.

[[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]]

The result is a freshly allocated list; element payloads are copied into it, so
the returned list does not share storage with `value`. `value` is not modified.
`collections::drop` is the complementary operation, returning what `take` leaves
behind.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The source list. Any length is accepted, including the empty list. Named-argument spelling is `value`. [[src/builtins/collections_package.mfb:__collections_take]] |
| `count` | `Integer` | How many leading elements to keep. Any `Integer` is accepted: values at or below 0 yield the empty list and values at or above the length yield the whole list. Named-argument spelling is `count`. [[src/builtins/collections_package.mfb:__collections_take]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the first `min(max(count, 0), len(value))` elements of `value`, in their original order. [[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The arena cannot allocate the result list, or its computed size overflows. No value of `count` is itself rejected. [[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]] [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Type checking

`T` is inferred from `value` and carries no ordering or comparability
requirement: `take` copies a contiguous range and never inspects an element, so
any list element type is accepted. `count` must be `Integer`.

## Examples

Keep the first two elements:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET head AS List OF Integer = collections::take([1, 2, 3, 4], 2)
  io::print(toString(len(head)))
  RETURN 0
END FUNC
```

An oversized count yields the whole list; a non-positive count yields an empty
one:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET all AS List OF Integer = collections::take([1, 2, 3], 99)
  LET none AS List OF Integer = collections::take([1, 2, 3], 0)
  io::print(toString(len(all)))
  io::print(toString(len(none)))
  RETURN 0
END FUNC
```

Split a list into a head and a tail with `take` and `drop`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET items AS List OF String = ["a", "b", "c", "d"]
  LET first AS List OF String = collections::take(items, 2)
  LET rest AS List OF String = collections::drop(items, 2)
  io::print(collections::get(rest, 0))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections drop`
- `mfb man collections mid`
- `mfb man collections chunks`
- `mfb man collections`
