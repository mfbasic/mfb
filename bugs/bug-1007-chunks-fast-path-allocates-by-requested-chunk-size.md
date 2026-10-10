# bug-1007: fixed-width `collections::chunks` fast path allocates each chunk at the requested size, even when it holds only a few elements

Last updated: 2026-10-07
Effort: small (<1h)
Severity: MEDIUM
Class: Other (memory / performance)

Status: Open
Regression Test: `tests/runtime/rt_collection_chunks_alloc_size.rs` (proposed; compare `--debug` allocation bytes for literal and dynamic sizes)

For fixed-width list elements and a compile-time `chunkSize`, the native `collections::chunks` fast path reserves every inner chunk block using the full requested size. A three-element list chunked with `1,000,000` therefore allocates about 8 MB for one inner block even though the returned chunk contains three elements and needs only a few dozen bytes. The result is correct, but an oversized chunk size can cause memory pressure or an out-of-memory error on a small input.

**Correct behavior:** allocation for each chunk should be proportional to the number of elements actually stored in it. `chunks([1, 2, 3], 1000000)` should have allocation cost comparable to chunking the same list with a runtime-provided size of 1000000.

## Failing Reproduction

Build this program with `mfb build -q --debug` and execute it:

```
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET xs = collections::chunks([1, 2, 3], 1000000)
  io::print(toString(len(xs)))
  io::print(toString(len(collections::get(xs, 0))))
  RETURN 0
END FUNC
```

Compare with the same call using a dynamic value:

```
IMPORT collections
IMPORT io
IMPORT os

FUNC main() AS Integer
  LET size AS Integer = toInt(os::getEnvOr("SIZE", "1000000"))
  LET xs = collections::chunks([1, 2, 3], size)
  io::print(toString(len(xs)))
  io::print(toString(len(collections::get(xs, 0))))
  RETURN 0
END FUNC
```

- Observed on macOS aarch64 with `target/debug/mfb` at HEAD `9d2240565`: both return one chunk of three elements, but the literal-size fast path reports `arena.0.alloc_bytes 8000288` and `peak_live_bytes 8000208`; the dynamic generic path reports `alloc_bytes 560` and `peak_live_bytes 464` (`mfb build -q --debug`; run each executable and inspect the `mfb.debug` report).
- Expected: both produce the same contents with allocation near the actual result size, independent of the unused remainder of `chunkSize`.

## Root Cause

`src/codegen/builtins/collections/func_chunks.rs`, `lower_collection_chunks_call`: `block_stride` is computed as `COLLECTION_HEADER_SIZE + size * 8`; `chunkCount * (COLLECTION_ENTRY_SIZE + block_stride)` is then allocated up front. Each inner chunk header records only its actual `chunkSize`, but chunk addresses are spaced by the full maximum stride. This preserves simple fixed-stride addressing at the cost of reserving the maximum chunk capacity for a short final chunk—and even when the entire input fits in one short chunk.

## Blast Radius

- Fixed-width native `chunks` fast path (`Integer`, `Float`, `Fixed`, `Money`) — observed; allocation is tied to the literal request.
- String `chunks` fast path — appears unaffected: it slices each actual range and grows the outer list with the resulting tight inner block (`lower_collection_chunks_string_call`); verify if the fixed-width path is redesigned.
- Generic/dynamic `chunks` body — contrast path; the reproduction allocates in proportion to actual elements.

## Fix Design

Size each chunk block from its actual element count, or otherwise avoid using the maximum requested size as every block's physical stride. Preserve correct inner-list headers, outer offsets, and drop/free behavior for the shared result allocation.

## Validation Plan

- Regression: allocation counters for the constant and dynamic reproductions remain within a small fixed overhead of each other; contents remain `[1, 2, 3]` in one chunk.
- Runtime proof: execute both debug builds and compare values plus arena allocation counters.
- Doc sync: none expected; behavior and returned values stay the same.
- Full suite: acceptance and runtime suites after implementation.
