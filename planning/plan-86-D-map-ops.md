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
- [ ] **D1 — `try_inplace_remove_key_assign`** (in-place compaction removeKey). **TOMBSTONE CONFIRMED
  INFEASIBLE as reuse (plan-86-A session scout); compaction is the viable ~2× win, NOT band-clearing.**
  Why no tombstone: the map index is **open addressing storing absolute entry indices** (`mod.rs:2407-2703`),
  the probe halts on `bucket==0` with **no DELETED sentinel**, the entry `FLAGS` bit is **read nowhere**
  (`builder_arena_transfer.rs:941-955` — the only former reader was deleted), and every `0..count` consumer
  (`keys`/`values`/`mapValues`/`merge`/transfer/build_buckets) assumes all entries live. A tombstone would need
  a DELETED bucket sentinel + probe change + bucket_put reuse + a live-count separate from array-COUNT + a
  USED-skip in every consumer + a compaction trigger — a structural project, out of scope.
  **Viable path = in-place COMPACTION** (mapchurn churn ~165 → ~80 ms, ~2×, stays P1 vs py O(1) dict delete;
  removes the alloc + the second O(N) copy pass + the fresh-map overhead, but STILL resets `BUCKETS_READY=0`
  so the next probe rebuilds — unavoidable given the index). **Edit points:** (1) new
  `try_inplace_remove_key_assign` in `builder_inplace_assign.rs` cloned from `try_inplace_set_add_assign:112-174`
  (match `native_builtin_target==Some("removeKey")`, `args[0]==name`; KEEP the `by_ref` guard + the live-FOR-EACH
  exclusion — a compaction shift IS observable to an iterator; require `map_type_parts` + probe-eligible key;
  materialize key to a slot); (2) wire into the assign chain `builder_control.rs:579-599`; (3) new
  `lower_map_remove_key_in_place` in `map_mutate.rs` (sibling of `lower_map_remove_key:1219`): single scan to
  find entry `i` via `emit_collection_payload_matches_value_branch:1292`; if found, shift entry table
  `[i+1..count)` down one 40-byte slot (pattern from `lower_list_prepend_in_place`, `list_mutate.rs:1574-1610`),
  optionally compact the data tail + fix shifted KEY/VALUE_OFFSETs (or leave data slack like `set` does),
  decrement COUNT(+8)/DATA_LENGTH(+24), store 0 into BUCKETS_READY(+4); no arena_alloc; (4) `local.constant=None`.
  Set `remove` comes free (routes through `lower_map_remove_key`, `collection_mutate.rs:464`). Baselines:
  churn 161.5 ms, checksum `2128750` (order-independent sum — compaction reorder is checksum-safe, but keys()/
  values() order IS observable elsewhere so PRESERVE insertion order: shift the tail, never swap-with-last).
  **NOTE:** `lower_list_remove_at` (`list_mutate.rs:2094`) is itself OUT-OF-PLACE (it allocs a result), so the
  in-place entry-table + data-tail shift is FRESH code — model it on the in-place shift in
  `lower_list_prepend_in_place` (`list_mutate.rs:1574-1610`), not on `remove_at`. The scan/match to find entry
  `i` reuses `emit_collection_payload_matches_value_branch` (`map_mutate.rs:1292`); the header-field offsets are
  COUNT+8 / DATA_LENGTH+24 / BUCKETS_READY+4 / entry stride 40 (`error_constants.rs:984-1019`). Trimmed
  measurement harness ready: `/tmp/bench-ld` main includes `test_mapchurn_churn`/`_iterate`/`test_map_str_ops`.
  **DESIGN NUANCE (decide + MEASURE both):** the simplest compaction shifts ONLY the entry table
  `[i+1..count)` down one 40-byte slot and leaves the removed entry's key/value bytes as DATA slack (shifted
  entries keep their absolute KEY/VALUE_OFFSETs — data isn't moved, so no offset fixup; matches how `set`
  leaves dead value slack). Correct + O(N)-entry-shift only. BUT under the 4000-cycle churn the slack grows
  (~2-4 B/removeKey) → `DATA_LENGTH` climbs toward `DATA_CAPACITY` → earlier reallocs, which may erode the ~2×.
  The alternative (shift the data tail too + fix the shifted entries' offsets) reclaims the slack at an extra
  O(dataLen) copy. Implement entry-only first (simplest), measure churn; if reallocs dominate, add data
  compaction. Either way COUNT-=1, BUCKETS_READY=0. The scan/match reuses
  `emit_collection_payload_matches_value_branch(key_type,"",map_ptr,key_off,key_len,query_key,no_match,match)`
  (`map_mutate.rs:1292`).
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
