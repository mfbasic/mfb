# chunks

Split a list into consecutive, non-overlapping blocks of at most `chunkSize` elements

## Synopsis

```
collections::chunks OF T(value AS List OF T, chunkSize AS Integer) AS List OF List OF T
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

`collections::chunks` walks `value` from index 0 in steps of `chunkSize`, and
for each step emits the range starting there and running `chunkSize` elements
forward, stopping early at the end of the list. The result is a list of those
blocks. It is a generic function written in MFBASIC source, rewritten to the
internal `__collections_chunks` generic and instantiated for the element type
`T` during monomorphization. [[src/builtins/collections.rs:internal_name]]
[[src/builtins/collections_package.mfb:__collections_chunks]]

Because the step and the block length are both `chunkSize`, the blocks are
consecutive and never overlap, and concatenating them reproduces `value`
exactly. Every block holds exactly `chunkSize` elements except possibly the
last: when the length of `value` is not a multiple of `chunkSize`, the final
block holds the remainder, which is between 1 and `chunkSize - 1` elements. No
padding element is ever inserted.
[[src/builtins/collections_package.mfb:__collections_chunks]]

An empty `value` produces an empty result — the loop never runs, so there is no
empty leading block. A `value` shorter than `chunkSize` produces exactly one
block holding the whole list.
[[src/builtins/collections_package.mfb:__collections_chunks]]

`chunkSize` must be at least 1. A `chunkSize` below 1 is rejected at runtime
with `ErrInvalidArgument`; there is no clamping and no default, so the argument
is always required. [[src/builtins/collections_package.mfb:__collections_chunks]]

Each block is built by the internal slice helper, which is lowered natively as a
bulk range copy, so element payloads are copied into freshly allocated lists and
no block shares storage with `value`. `value` is not modified.
[[src/target/shared/code/builder_collection_queries.rs:try_inline_slice_op]]
[[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to split. Any length is accepted; the empty list yields an empty result. Named-argument spelling is `value`. [[src/builtins/collections_package.mfb:__collections_chunks]] |
| `chunkSize` | `Integer` | The block length and the step between blocks. Must be 1 or greater; there is no default. Named-argument spelling is `chunkSize`. [[src/builtins/collections_package.mfb:__collections_chunks]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF List OF T` | The consecutive blocks of `value` in order, each of length `chunkSize` except possibly a shorter final block. The empty list when `value` is empty. Its length is `len(value)` divided by `chunkSize`, rounded up. [[src/builtins/collections_package.mfb:__collections_chunks]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `chunkSize` is less than 1. [[src/builtins/collections_package.mfb:__collections_chunks]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The arena cannot allocate a block or the result list, or a computed size overflows. [[src/target/shared/code/builder_collection_queries.rs:lower_list_slice_range]] [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Type checking

`T` is inferred from `value` and carries no ordering or comparability
requirement: `chunks` copies contiguous ranges and never inspects an element, so
any list element type is accepted. `chunkSize` must be `Integer`. The result
type is one level more nested than the argument: `List OF List OF T`.

## Examples

Split five elements into blocks of two, leaving a short final block:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET parts AS List OF List OF Integer = collections::chunks([1, 2, 3, 4, 5], 2)
  io::print(toString(len(parts)))
  io::print(toString(len(collections::get(parts, 2))))
  RETURN 0
END FUNC
```

A list shorter than the chunk size yields a single block:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET one AS List OF List OF Integer = collections::chunks([1, 2], 10)
  io::print(toString(len(one)))
  RETURN 0
END FUNC
```

Reject a non-positive chunk size at runtime:

```
IMPORT collections
IMPORT io
IMPORT errorCode

FUNC main AS Integer
  LET bad AS List OF List OF Integer = collections::chunks([1, 2, 3], 0) TRAP(e)
    io::print(toString(e.code = errorCode::ErrInvalidArgument))
    RECOVER []
  END TRAP
  RETURN 0
END FUNC
```

## See also

- `mfb man collections window`
- `mfb man collections take`
- `mfb man collections flatten`
- `mfb man collections`
