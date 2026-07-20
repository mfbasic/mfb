# window

Produce the sliding windows of a list, each of exactly `size` elements

## Synopsis

```
collections::window OF T(value AS List OF T, size AS Integer, stride AS Integer = 1) AS List OF List OF T
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

`collections::window` walks `value` from index 0 in steps of `stride`, and at
each position where a full run of `size` consecutive elements still fits, emits
that run as a window. The result is the list of those windows, in order. It is a
generic function written in MFBASIC source, rewritten to the internal
`__collections_window` generic and instantiated for the element type `T` during
monomorphization. [[src/builtins/collections.rs:internal_name]]
[[src/builtins/collections_package.mfb:__collections_window]]

Every window has exactly `size` elements — there is no short final window. The
loop advances only while `i + size` is still within the length of `value`, so a
trailing partial run is simply not emitted, and the elements it would have
contained are dropped from the result. This is the key difference from
`collections::chunks`, which does emit a short final block.
[[src/builtins/collections_package.mfb:__collections_window]]

`stride` controls the overlap and defaults to 1, so the common call
`collections::window(value, size)` produces maximally overlapping windows that
advance one element at a time. A `stride` equal to `size` produces
non-overlapping windows, and a `stride` greater than `size` skips elements
between them. [[src/builtins/collections_package.mfb:__collections_window]]

When `size` is greater than the length of `value`, no window fits and the result
is the empty list; an empty `value` likewise produces an empty result. Both
`size` and `stride` must be at least 1, and either being below 1 is rejected at
runtime with `ErrInvalidArgument`. Note the parameter is named `stride`, not
`step`. [[src/builtins/collections_package.mfb:__collections_window]]

Each window is built by the internal slice helper, which is lowered natively as
a bulk range copy, so element payloads are copied into freshly allocated lists
and no window shares storage with `value`. Overlapping windows therefore hold
independent copies of the elements they share. `value` is not modified.
[[src/target/shared/code/builder_collection_queries.rs:try_inline_slice_op]]
[[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to window over. Any length is accepted; a list shorter than `size` yields an empty result. Named-argument spelling is `value`. [[src/builtins/collections_package.mfb:__collections_window]] |
| `size` | `Integer` | The number of elements in each window. Must be 1 or greater; there is no default. Named-argument spelling is `size`. [[src/builtins/collections_package.mfb:__collections_window]] |
| `stride` | `Integer` | How many positions to advance between consecutive windows. Must be 1 or greater. Defaults to 1 when omitted. Named-argument spelling is `stride`. [[src/builtins/collections_package.mfb:__collections_window]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF List OF T` | The sliding windows of `value` in order, each holding exactly `size` elements, starting at indexes 0, `stride`, `2 * stride`, and so on for as long as a full window fits. The empty list when `size` exceeds `len(value)`. [[src/builtins/collections_package.mfb:__collections_window]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `size` is less than 1, or `stride` is less than 1. [[src/builtins/collections_package.mfb:__collections_window]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The arena cannot allocate a window or the result list, or a computed size overflows. [[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]] [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Type checking

`T` is inferred from `value` and carries no ordering or comparability
requirement: `window` copies contiguous ranges and never inspects an element, so
any list element type is accepted. `size` and `stride` must both be `Integer`.
The result type is one level more nested than the argument: `List OF List OF T`.

## Examples

Overlapping pairs, with the default stride of 1:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET windows AS List OF List OF Integer = collections::window([1, 2, 3, 4], 2)
  io::print(toString(len(windows)))
  RETURN 0
END FUNC
```

A stride equal to the size gives non-overlapping windows — and unlike `chunks`,
a trailing partial run is dropped:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET pairs AS List OF List OF Integer = collections::window([1, 2, 3, 4, 5], 2, 2)
  io::print(toString(len(pairs)))
  RETURN 0
END FUNC
```

Name the stride explicitly; the parameter is `stride`, not `step`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET spaced AS List OF List OF Integer = collections::window([1, 2, 3, 4, 5, 6], 2, stride := 3)
  io::print(toString(len(spaced)))
  RETURN 0
END FUNC
```

A size larger than the list yields no windows at all:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET none AS List OF List OF Integer = collections::window([1, 2], 5)
  io::print(toString(len(none)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections chunks`
- `mfb man collections take`
- `mfb man collections zip`
- `mfb man collections`
