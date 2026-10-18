# bug-1011: reserved collection results retain source-sized capacity when outputs are smaller

Last updated: 2026-10-07
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory use / performance)

Status: Open
Regression Test: `tests/runtime/rt_reserved_collection_result_memory.rs` (proposed)

## Cause (coordinator, bug-staging-24)

- OWNER RULING 2026-10-18 (one rule for 1009, 1010, 1011): reserve then shrink. A builder whose input only bounds its result keeps its one up-front reservation at that bound, and before it returns it shrinks the result to exactly what it holds (`capacity == count`, `dataCapacity == dataLength`, buckets re-sized for a Map/Set), so nothing input-sized outlives the call. `mfb spec memory` § Capacity Headroom and Growth changes its "allocates once at that bound ... leaves `count < capacity` in the result" sentence to this rule.
- Where: the native fast paths that reserve from the input: `func_to_set.rs:lower_collection_to_set_call` (1009), `func_merge.rs:lower_collection_merge_call` / `copy_map_with_capacity` (1010), `gen_mutate.rs:lower_reserved_list` called by `lower_filter` / `lower_transform` (1011).
- Fix: one shared shrink-to-fit step (the tightening `copy_flat_block` already performs for value copies: an in-place trim or one tight copy-and-free) applied at each of those builders' return, not a per-builder variant.

## Failing Reproduction

Build and run this project with `target/debug/mfb build --debug <project>`:

```
IMPORT collections
IMPORT io
FUNC reject(value AS String) AS Boolean
  RETURN FALSE
END FUNC
FUNC main() AS Integer
  MUT xs AS List OF String = []
  FOR i = 0 TO 49999
    xs = collections::append(xs, "x")
  NEXT
  LET ys = collections::filter(xs, reject)
  io::print(toString(len(ys)))
  RETURN 0
END FUNC
```

- Observed on macOS aarch64: prints `0`; `arena.0.alloc_bytes` is `13189440`.
- Contrast baseline: remove the filter call and print `len(xs)`; it prints `50000` and `arena.0.alloc_bytes` is `10339392`.
- Expected: the filtered result is empty and does not retain an output buffer sized for all 50,000 source strings.

The two runs use identical input construction, so the measured 2,850,048-byte allocation delta is attributable to the filter result reservation. `arena.0.peak_live_bytes` is equal in these runs because the input-building loop reaches a higher peak; the allocation counter exposes the extra result block.

Transform payloads can also shrink while preserving the element count:

~~~
IMPORT collections
IMPORT io
IMPORT strings
FUNC blank(value AS String) AS String
  RETURN \"\"
END FUNC
FUNC main() AS Integer
  LET long AS String = strings::repeat(\"x\", 4096)
  LET xs AS List OF String = collections::repeat([], long, 1000)
  LET ys = collections::transform(xs, blank)
  io::print(toString(len(ys)))
  RETURN 0
END FUNC
~~~

- Observed on macOS aarch64: prints `1000`; `arena.0.peak_live_bytes` is `8280336`.
- Contrast baseline: replace the transform call with printing `len(xs)`; it prints `1000` and peak live bytes are `4140256`.
- Expected: the result has 1,000 empty strings and does not reserve the source's 4,096,000 payload bytes.

The output retains one entry per input, so the entry table is required; the extra data reservation is not, because every transformed String is empty.

`groupBy` inherits the transform reservation in both projected lists:

~~~
IMPORT collections
IMPORT io
IMPORT strings
FUNC strLen(value AS String) AS Integer
  RETURN len(value)
END FUNC
FUNC blank(value AS String) AS String
  RETURN ""
END FUNC
FUNC main() AS Integer
  LET long AS String = strings::repeat("x", 4096)
  LET xs AS List OF String = collections::repeat([], long, 1000)
  LET groups = collections::groupBy(xs, strLen, blank)
  io::print(toString(len(groups)))
  RETURN 0
END FUNC
~~~

- Observed on macOS aarch64: prints `1`; `arena.0.peak_live_bytes` is `12412912`.
- Contrast baseline: print `len(xs)` without calling `groupBy`; peak live bytes are `4140256`.
- Expected: projected key and value lists should size their payload storage to the Integer keys and empty Strings they produce, not the 4,096,000-byte source payload.

## Root Cause

Inspection with `rg -n 'lower_reserved_list|Pre-size the output' src/codegen/builtins/collections/func_filter.rs src/codegen/builtins/collections/func_transform.rs src/codegen/builtins/collections/gen_mutate.rs` shows both `lower_filter` and `lower_transform` call the helper before producing any values. `lower_reserved_list` sets result `capacity = source.count` and `dataCapacity = source.dataLength`, then allocates the full block. Filter can reject entries, leaving both reservations unused. Transform emits one entry per input, so its entry capacity is right, but shorter payloads leave the source-sized data reservation unused.

## Goal

- Preserve callback order, error propagation, result order, and result contents while sizing sparse filter entries and transformed payloads to the results they actually produce.

### Non-goals (must NOT change)

- Predicate call count/order or callback-error propagation.
- Filtered element copying/ownership semantics or source immutability.
- Transform's entry count: it still emits one element per input, though output payload capacity can be smaller.

## Blast Radius

- `collections::filter` (`lower_filter`) — affected; measured for a String list and an always-false predicate.
- `collections::transform` (`lower_transform`) — affected for output types whose payloads are smaller than the source data length; measured with String-to-empty-String.
- `collections::sort` — unaffected; its copied result preserves all source elements and payload bytes.
- String `collections::sortBy` gather path — affected transitively; it calls `transform` to build key strings, so short projected keys can leave a source-sized data reservation in the temporary key list.
- `collections::groupBy` — affected transitively; its key and value projections use `transform`, so shorter projected payloads can reserve the original list's full data length in temporary lists.

## Fix Design

Avoid reserving all source payload bytes and entries for operations whose outputs may be smaller. Incremental growth follows actual kept entries and produced bytes; alternatively, preserve one-allocation behavior with a bounded or staged reservation. Measure sparse filters and payload-shrinking transforms alongside all-kept and same-size cases.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a debug-arena test for empty, sparse, and all-kept filters, transforms whose output payload is smaller/equal/larger, and String `sortBy` with short projected keys.
- [ ] Confirm callback order, callback failure cleanup, and result ownership still hold for the alternate sizing path.

Acceptance: the excess-reservation cases are reproduced; callback and result semantics are correct; the audit confirms which callers preserve source size and which can shrink output.
Commit: —

### Phase 2 — the fix

- [ ] Change reserved output sizing so unused input entries and payload bytes are not retained for smaller results.
- [ ] Preserve callback failure cleanup and error propagation.

Acceptance: empty/sparse filters allocate in proportion to kept elements; shrinking transforms do not reserve source-sized payload space; equal/larger transforms remain correct.
Commit: —

### Phase 3 — full validation

- [ ] Run the reserved-result memory regression, String `sortBy` tests and callback-failure runtime tests.
- [ ] Run the full test suite.

Acceptance: memory bounds hold for sparse filters and shrinking transforms, existing callback behavior remains unchanged, and the full suite passes.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_reserved_collection_result_memory.rs`.
- Runtime proof: compare arena allocation and peak-live counters for sparse filters and payload-shrinking transforms.
- Doc sync: none expected; this changes allocation strategy only.
- Full suite: runtime and full Cargo test suites.

## Open Decisions

- Exact one-pass growth versus a bounded initial reservation — measure sparse and all-kept filters, plus shrinking and growing transforms; smaller outputs must not retain source-sized capacity.

## Summary

The excess allocation comes from the shared reserved-list helper's assumption that source data length is a good output data-capacity bound. The main risk is reducing retained memory without making all-kept filters or transforms allocate excessively or weakening callback cleanup.
