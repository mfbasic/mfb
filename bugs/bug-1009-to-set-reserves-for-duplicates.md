# bug-1009: fixed-width `collections::toSet` reserves by input length even when nearly every element is a duplicate

Last updated: 2026-10-07
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory use / performance)

Status: Open
Regression Test: `tests/runtime/rt_to_set_duplicate_memory.rs` (proposed)

The fixed-width `collections::toSet` fast path allocates its result with capacity equal to the input list's length before it checks whether any elements are duplicates. In measurements from `target/debug/mfb build --debug /tmp/mfb_collections_audit` followed by running `/tmp/mfb_collections_audit/build/collection_audit.out`, a list of 100,000 copies of `7` produces a one-element set but raises peak live arena memory from 1,771,536 bytes for the input alone to 8,262,976 bytes. An incremental `collections::add` loop over the same input returns the same one-element set with peak memory unchanged at 1,771,536 bytes. On a memory-constrained process, this avoidable reservation can make `toSet` fail with `ErrOutOfMemory` even though the result is small.

**Correct behavior:** the fixed-width conversion must not reserve storage proportional to the number of input elements when the result contains far fewer distinct values. Peak memory for duplicate-heavy inputs should track the number of distinct set entries, in addition to storage already occupied by the input.

References:

- `src/codegen/builtins/collections/func_to_set.rs` — native fast path and generic body.
- `src/docs/spec/memory/05_collections.md`, “Capacity Headroom and Growth” — spare capacity is an implementation detail; it does not require reserving the input's upper bound for a set conversion.
- Confirmed on macOS aarch64 at the current worktree HEAD with `target/debug/mfb build --debug` and the runtime arena counters.

## Failing Reproduction

Build and run this project with `target/debug/mfb build --debug <project>`:

```
IMPORT collections
IMPORT io
FUNC main() AS Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO 99999
    xs = collections::append(xs, 7)
  NEXT
  LET s = collections::toSet(xs)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

- Observed on macOS aarch64: prints `1`; `arena.0.alloc_bytes` is `10381504` and `arena.0.peak_live_bytes` is `8262976`.
- Expected: it still prints `1`, while the conversion's storage scales with the one distinct value rather than reserving 100,000 set slots.

Contrast: replace the `LET s` line with `MUT s AS Set OF Integer = Set OF Integer {}` and add `FOR EACH x IN xs / s = collections::add(s, x) / NEXT`. The result is also one element, with `arena.0.alloc_bytes` `3181840` and `arena.0.peak_live_bytes` `1771536`. The input-building code is identical, so the extra allocation and peak live memory come from `toSet`'s result reservation.

## Root Cause

Source inspection via `rg -n 'lower_collection_to_set_call|to_set_native_width|__collections_toSet' src/codegen/builtins/collections/func_to_set.rs` confirms that `lower_collection_to_set_call` reads `n = len(value)` and allocates `HEADER + n*ENTRY + n*stride + n*16` bytes for entries, payload and hash buckets before probing any key. It writes `capacity = n` and `dataCapacity = n*stride`, then inserts each input item. Repeated keys leave the reservation unused. The generic `__collections_toSet` body instead starts empty and calls `collections::add` for each item, so the set grows with the distinct count.

## Goal

- Preserve result contents and first-occurrence order while preventing a duplicate-heavy fixed-width input from causing result capacity and peak live memory to scale with the input count.

### Non-goals (must NOT change)

- Set membership, deduplication, iteration order, or fixed-width key equality (including bitwise Float key equality).
- `collections::toSet` behavior for variable-width and `Nothing` element types.
- The source list or its capacity.

## Blast Radius

- Fixed-width `collections::toSet` fast path (`Integer`, `Float`, `Fixed`, `Money`, `Byte`, `Boolean`, `Scalar`) — affected; measured for `Integer` and shared `lower_collection_to_set_call` implementation.
- Generic `__collections_toSet` fallback for variable-width values and `Nothing` — unaffected; it incrementally adds values.
- `collections::add` and Set literals — unaffected; they insert incrementally and already collapse duplicates.

## Fix Design

Keep duplicate detection and first-occurrence ordering, but avoid allocating the full upper bound before discovering cardinality. A one-pass growing set follows the existing generic path's memory behavior; an alternative must retain the same bound without materializing a full-size result for duplicate-heavy inputs. Check unique-heavy performance as a tradeoff, but do not accept O(input length) spare result capacity for a one-element output.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a `--debug` runtime test comparing a duplicate-heavy `toSet` with the equivalent incremental-add result and assert the result length and bounded peak live memory.
- [ ] Measure a unique-heavy input too, so the test captures the memory/performance tradeoff and the audit confirms all fixed-width element specializations use this lowering.

Acceptance: the regression demonstrates the current excess reservation, both results contain the same first-occurrence-ordered values, and the blast-radius audit is complete.
Commit: —

### Phase 2 — the fix

- [ ] Change `lower_collection_to_set_call` so result capacity grows with distinct entries rather than input count.
- [ ] Preserve Float bitwise equality and first-occurrence order.

Acceptance: duplicate-heavy peak memory stays bounded by input storage plus memory for the distinct result; unique-heavy conversion remains correct.
Commit: —

### Phase 3 — full validation

- [ ] Run the regression at multiple duplicate ratios and the existing collection runtime tests.
- [ ] Run the full test suite.

Acceptance: all results match the existing `toSet` contract and the memory bound holds without regressions.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_to_set_duplicate_memory.rs`.
- Runtime proof: compare debug arena counters for duplicate-heavy and unique-heavy inputs and verify values/order.
- Doc sync: none expected; this changes allocation strategy only.
- Full suite: runtime and full Cargo test suites.

## Open Decisions

- Unique-heavy throughput versus duplicate-heavy memory — use the one-pass grow path unless measurements justify another design that avoids the current upper-bound allocation.

## Summary

The issue is isolated to the fixed-width `toSet` accelerator's up-front reservation. The generic body provides a behavior-equivalent contrast and a straightforward memory bound; implementation risk is retaining the accelerator's unique-heavy performance without keeping the duplicate-heavy allocation spike.
