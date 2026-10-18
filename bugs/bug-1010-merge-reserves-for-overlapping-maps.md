# bug-1010: native `collections::merge` reserves for every input entry even when all keys overlap

Last updated: 2026-10-07
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory use / performance)

Status: Open
Regression Test: `tests/runtime/rt_merge_overlap_memory.rs` (proposed)

## Cause (coordinator, bug-staging-24)

- WAITING ON THE OWNER (asked 2026-10-17). `mfb spec memory` § Capacity Headroom and Growth states the opposite of this doc's Correct behavior ("A builder whose input only bounds its result allocates once at that bound: native `toSet` ... reserves `capacity = len(value)`"), and no owner ruling is recorded. Options put to the owner: (a) size to the result (grow as the generic path does), (b) reserve at the bound then shrink-to-fit before returning (recommended), (c) keep and close as intended. One rule covers 1009, 1010 and 1011.

In measurements from `target/debug/mfb build --debug /tmp/mfb_collections_audit` followed by running `/tmp/mfb_collections_audit/build/collection_audit.out`, the native `collections::merge(a, b, TRUE)` fast path reserves capacity for `len(a) + len(b)` entries and both maps' payload bytes before checking key collisions. When the maps contain the same 10,000 String keys, the result contains only 10,000 entries, but the presized path allocates 720,000 more bytes and raises peak live arena memory by the same amount compared with an equivalent sequence of `collections::set` calls. The reservation is for entries that cannot be appended because every key from `b` overwrites a key already in `a`; on a memory-constrained process, this can cause avoidable `ErrOutOfMemory`.

**Correct behavior:** the String-key/fixed-width-value accelerator must not reserve space for all of `b` when its keys overlap with `a`; result memory should track the union of keys and the payloads actually written.

References:

- `src/codegen/builtins/collections/func_merge.rs` — native fast path and its presized copy.
- `src/docs/spec/stdlib/05_collections.md` — `merge` returns the union of the two key sets; collisions replace values without adding entries.
- Confirmed on macOS aarch64 at the current worktree HEAD with `target/debug/mfb build --debug` and the runtime arena counters.

## Failing Reproduction

Build and run this project with `target/debug/mfb build --debug <project>`:

```
IMPORT collections
IMPORT io
FUNC main() AS Integer
  MUT a AS Map OF String TO Integer = Map OF String TO Integer {}
  MUT b AS Map OF String TO Integer = Map OF String TO Integer {}
  FOR i = 0 TO 9999
    LET k AS String = toString(i)
    a = collections::set(a, k, i)
    b = collections::set(b, k, i + 1)
  NEXT
  LET m = collections::merge(a, b, TRUE)
  io::print(toString(len(m)))
  RETURN 0
END FUNC
```

- Observed on macOS aarch64: prints `10000`; `arena.0.alloc_bytes` is `6761536` and `arena.0.peak_live_bytes` is `3188896`.
- Expected: it still prints `10000`, with no reservation for duplicate entries from `b`.

Contrast: replace the `LET m` line with `MUT m AS Map OF String TO Integer = a` and add `FOR EACH e IN b / m = collections::set(m, e.key, e.value) / NEXT`. This produces the same union and values; it measures `arena.0.alloc_bytes` `6041536` and `arena.0.peak_live_bytes` `2468896`. The maps are constructed identically, so the `merge` fast path adds 720,000 bytes of allocation and peak live memory for its unused reservation.

## Root Cause

Source inspection via `rg -n 'copy_map_with_capacity|extra_count = b.count|extra_data = b.dataLength|lower_collection_merge_call' src/codegen/builtins/collections/func_merge.rs` confirms that `lower_collection_merge_call` sets `extra_count = b.count` and `extra_data = b.dataLength` unconditionally. `copy_map_with_capacity` uses those values to reserve `a.count + b.count` entries and `a.dataLength + b.dataLength` data bytes, plus buckets sized for that capacity. The subsequent loop inserts each `b` entry through `lower_map_set_in_place`; when a key already exists, it overwrites the value and does not consume the reserved entry or key-payload space. The generic `__collections_merge` body starts with a copy of `a` and grows only when `b` contributes a previously absent key.

## Goal

- Preserve `preferB = TRUE` values and traversal order while avoiding reservation of entries and payload bytes for keys that already exist in `a`.

### Non-goals (must NOT change)

- `preferB` collision semantics, map key equality, map iteration order, or either input map.
- Fast-path coverage for other key/value types or `preferB = FALSE`.
- The map's serialized/runtime layout.

## Blast Radius

- `lower_collection_merge_call` for String-key/fixed-width-value maps and literal `preferB = TRUE` — affected; measured for `String TO Integer` with identical key sets.
- Other `merge` instantiations — unaffected by this native reservation path; they use the generic source implementation.
- `collections::set` and map literals — unaffected; they insert only the keys supplied.

## Fix Design

Avoid allocating the full sum of both input maps before resolving collisions. Options include counting keys absent from `a` and the bytes they contribute before presizing, or using incremental growth as the generic body does. Preserve the current fast path's unique-key behavior where possible, but prioritize avoiding peak memory proportional to overlap that does not enter the result.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a debug-arena regression comparing `merge(a, b, TRUE)` with an equivalent `set` loop for identical, disjoint, and partially overlapping keys.
- [ ] Verify values, length, iteration order, and allocation bounds for each overlap ratio.

Acceptance: the current excessive reservation is demonstrated, and the fast-path/type gate and overlap cases are fully audited.
Commit: —

### Phase 2 — the fix

- [ ] Size the result for keys and payloads that actually enter the union, or switch to incremental growth.
- [ ] Preserve collision replacement and key order.

Acceptance: fully overlapping inputs do not reserve a second copy of their entries; disjoint inputs still merge correctly.
Commit: —

### Phase 3 — full validation

- [ ] Run the allocation regression for all overlap ratios and `tests/runtime/rt_merge_fast_path_frees.rs`.
- [ ] Run the full test suite.

Acceptance: all merge semantics remain unchanged, measured memory follows the union result, and the full suite passes.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_merge_overlap_memory.rs`.
- Runtime proof: compare arena counters for fully overlapping, partially overlapping and disjoint input maps.
- Doc sync: none expected; this changes allocation strategy only.
- Full suite: runtime and full Cargo test suites.

## Open Decisions

- Exact presizing versus incremental growth — compare partial-overlap costs; either design must avoid reserving the entire second map when its keys all collide.

## Summary

The excess allocation is isolated to the optimized `preferB = TRUE` map merge path. The generic source implementation provides a correct contrast; the main implementation risk is preserving the accelerator's performance for disjoint maps while reducing memory for overlapping maps.
