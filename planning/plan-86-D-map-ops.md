# plan-86-D — map in-place removeKey + String mapValues + native merge

Sub-plan **D** of [plan-86](plan-86-benchmark-perf.md). Open.

**Covers (3 P1 + matrix):** mapchurn churn (165), iterate (14.27), map str_ops (5.97); plus the mfb-only
removeKey matrix rows (`map (State-Dynamic) removeKey` 62.6, `State-Fixed` 17.3).

## Root cause
- **removeKey has no in-place path** — `m = removeKey(m,k)` → `lower_map_remove_key` (`map_mutate.rs:1219`):
  O(N) survivor scan + fresh `arena_alloc` + O(N) entry copy, and the fresh header resets `BUCKETS_READY=0`
  (`collection_buffer.rs:190`) so the next `set`'s probe rebuilds the whole index — **two O(N) passes per
  cycle**. The State/record rows scale with total map bytes.
- **merge deep-copies the base:** `__collections_merge` opens `MUT result = a` → owner-copy deep-copies the
  whole base map each call.
- **String `mapValues` not native:** gate `builder_values.rs:788-790` allows only 8-byte values.

## Fixes
- [ ] **D1 — `try_inplace_remove_key_assign`** (mirrors C2/list-append-in-place): delete the entry in place
  (compact the data tail + unlink from its bucket incrementally), keep `BUCKETS_READY=1`, no alloc/copy.
  ~80-120 LOC; register-spill discipline (`[[arena-alloc-clobbers-x14-x15]]`). **Measure behind a toggle
  first** — the incremental bucket unlink must not cost more than it saves. **NOTE (this session's
  analysis):** insertion-order preservation forces O(N) data compaction, so D1 is a ~2× constant-factor win
  that still loses to Python's O(1) dict delete (mapchurn churn stays P1) — clearing the row likely needs a
  **tombstone-based delete** (O(1) + periodic compaction), a bigger design than "compact the data tail".
  Reconsider the approach before implementing.
- [ ] **D2 — native String-value `mapValues`** (variable-width same-type path: copy the key/bucket structure,
  rebuild only value payloads keeping `ready=1`). Helps map str_ops. **NOTE (plan-86-A session) — MEASURE
  first; likely MODEST, not groupBy-class.** The scored row is `map str_ops` at only ~5.97 ms (py 2.82), NOT
  a 166 ms O(container)-copy row like groupby: mapValues values are single small Strings, not 500-element
  buckets, so the per-entry `set` cost is small (and the C2-style in-place-set may already fire on
  `result = set(result, e.key, ...)`). The fixed-width native `mapValues` (`lower_collection_map_values_call`,
  gate `#collections_mapValues$K$V$U`, V==U fixed-width) copies the map once + rewrites value payloads IN
  PLACE — but that only works because fixed-width values are same-size; a String `f(value)` changes size, so
  the in-place rewrite does NOT apply and a String D2 must rebuild the value data region (build a new map,
  `lower_map_set_in_place(result, key, f(value))` per entry). Expect a few-ms win, not a 445× drop.
- [ ] **D3 — native `merge`** (size to `|a|+|b|`, copy `a` once with buckets built, bulk-insert both). Modest
  (base copy inherent to value semantics) — lowest ROI. Helps mapchurn iterate.

## Acceptance
map/mapchurn checksums (catch bucket corruption as a wrong lookup) + `cargo test` + map fixtures +
`scripts/artifact-gate.sh`.
