# drop

Return a new list with the first `count` elements removed

## Synopsis

```
collections::drop OF T(value AS List OF T, count AS Integer) AS List OF T
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

`collections::drop` returns a new list containing everything in `value` except
its leading `count` elements, in their original order. It is a generic function
written in MFBASIC source: the call is rewritten to the internal
`__collections_drop` generic and instantiated for the element type `T` during
monomorphization. [[src/builtins/collections.rs:internal_name]]
[[src/builtins/collections_package.mfb:__collections_drop]]

`drop(value, count)` is defined as the half-open range `[count, len(value))` of
`value`, delegated to the internal slice helper. That helper is lowered natively
as a bulk range copy, and the native lowering is what defines the boundary
behavior: the range start is clamped into `[0, len]` and the range stop into
`[start, len]`. [[src/builtins/collections_package.mfb:__collections_drop]]
[[src/target/shared/code/builder_collection_queries.rs:try_inline_slice_op]]
[[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]]

That clamping makes `drop` **total** — every `Integer` value of `count` is
accepted and no index is ever rejected:

- `count` of 0 or any negative value clamps the start back to 0, so the whole
  list is returned.
- `count` greater than or equal to the length of `value` clamps the start to the
  length, so the result is the empty list.
- Otherwise the result holds `len(value) - count` elements.

[[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]]

The result is a freshly allocated list; element payloads are copied into it, so
the returned list does not share storage with `value`. `value` is not modified.
`collections::take` is the complementary operation, returning the elements
`drop` discards.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The source list. Any length is accepted, including the empty list. Named-argument spelling is `value`. [[src/builtins/collections_package.mfb:__collections_drop]] |
| `count` | `Integer` | How many leading elements to discard. Any `Integer` is accepted: values at or below 0 return the whole list and values at or above the length return the empty list. Named-argument spelling is `count`. [[src/builtins/collections_package.mfb:__collections_drop]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the last `len(value) - min(max(count, 0), len(value))` elements of `value`, in their original order. [[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The arena cannot allocate the result list, or its computed size overflows. No value of `count` is itself rejected. [[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]] [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Type checking

`T` is inferred from `value` and carries no ordering or comparability
requirement: `drop` copies a contiguous range and never inspects an element, so
any list element type is accepted. `count` must be `Integer`.

## Examples

Discard the first two elements:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET tail AS List OF Integer = collections::drop([1, 2, 3, 4], 2)
  io::print(toString(len(tail)))
  RETURN 0
END FUNC
```

An oversized count yields the empty list; a non-positive count yields the whole
list:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET none AS List OF Integer = collections::drop([1, 2, 3], 99)
  LET all AS List OF Integer = collections::drop([1, 2, 3], 0)
  io::print(toString(len(none)))
  io::print(toString(len(all)))
  RETURN 0
END FUNC
```

Skip a header row before processing the rest:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET rows AS List OF String = ["name", "ada", "grace"]
  LET body AS List OF String = collections::drop(rows, 1)
  io::print(collections::get(body, 0))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections take`
- `mfb man collections mid`
- `mfb man collections removeAt`
- `mfb man collections`
