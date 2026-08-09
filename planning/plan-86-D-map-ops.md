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
- [x] **D1 — `try_inplace_remove_key_assign`** (in-place entry-table compaction removeKey). **LANDED —
  mapchurn churn 161.5 → 21.96 ms (~7.3×, BETTER than the predicted ~2×), checksum `2128750` unchanged;
  stays P1 (py 1.20) but a big win.** `try_inplace_remove_key_assign` (`builder_inplace_assign.rs`, wired into
  the `builder_control.rs:592` assign chain after set-assign) recognizes `name = collections::removeKey(name,k)`
  on a uniquely-owned MUT map local (by_ref + live-FOR-EACH guards) and calls new
  `lower_map_remove_key_in_place` (`map_mutate.rs`): single scan for entry `i` (reuse
  `emit_collection_payload_matches_value_branch`; **note the arg7=on-MATCH / arg8=on-NO-MATCH order — a swap
  removed the wrong entry until fixed**), then shift the entry table `[i+1..count)` down one 40-byte slot
  (forward word copy), COUNT-=1, BUCKETS_READY=0 — NO arena_alloc, NO data copy, NO fresh map. The removed
  entry's key/value bytes are left as DATA SLACK — the SAME dead-slack pattern `lower_map_set_in_place`
  already uses when overwriting a value (`map_mutate.rs:8`, "old value becomes dead slack, tightened on
  copy"), reclaimed on the next tight copy (bind/return) — so it is NOT a new leak, just more frequent slack
  than set-overwrite. (A data-compaction refinement — shift the data tail + fix shifted offsets — would
  reclaim slack eagerly at an extra O(dataLen) copy; not needed for correctness.) Native/`.mfb` byte-identical
  across middle/first/last removal, no-op on a missing key, hasKey/get + re-add (bucket rebuild), Integer key,
  preserved insertion order, and the 200-cycle churn. Fixture: `map-removekey-inplace-rt`. Commit: `480e1c1f7`.
  Original analysis (kept for context): **TOMBSTONE CONFIRMED INFEASIBLE as reuse (plan-86-A session scout).**
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
  **FURTHER (plan-86-A session, confirmed by reading `lower_collection_map_values_call:3463`):** the
  fixed-width path only works via `copy_collection_tight` + same-size in-place value rewrite, which String
  cannot use. A String rebuild via per-entry `lower_map_set_in_place` is the SAME loop the `.mfb` already runs
  (`result = set(result, e.key, f(e.value))`), and that `set` almost certainly already fires the C2-style
  in-place set on the uniquely-owned MUT `result` — so native D2's only gain is the interpreted-loop overhead,
  i.e. MARGINAL like chunks/window/zip (`[[native-string-hof-rewrites-are-marginal]]`), NOT a groupBy-class
  win. **DEPRIORITIZE: measure the interpreted map str_ops loop first; likely not worth a ~100-line native
  lowering.** The valuable part of sub-plan D was D1 (removeKey, 7.3×, LANDED). D2/D3 are marginal/modest.
- [x] **D3 — native `merge`** — **LANDED. `mapchurn iterate` 14.27 → 9.27 ms (~36 %, ~5ms cut as predicted;
  stays P1 vs py 7.55 but the merge slice of the row is gone).** Gated to a String-key, fixed-width-value map
  (`#collections_merge$String$<Integer|Float|Fixed|Money>`) with a **compile-time `TRUE` preferB**
  (`args[2] == Const Boolean "true"`); other shapes (non-const/false preferB, String value, non-String key)
  fall through to the `.mfb __collections_merge`. Implementation: `copy_map_with_capacity` (presized copy variant
  — bigger alloc + `emit_write_collection_header_full` writing `capacity = a.count+b.count`,
  `dataCapacity = a.dataLength+b.dataLength`) then a loop over `b`'s entries doing `lower_map_set_in_place`
  (no grow — presized). preferB=TRUE ⇒ b overwrites a on a shared key == set_in_place's overwrite-or-append,
  so **no hasKey probe needed** (that's why the const-TRUE gate). **GOTCHA that SIGSEGV'd first:** the map
  stores keys as RAW bytes in the data region with the length in `entry.KEY_LENGTH`, but `set_in_place`
  expects a length-prefixed String value (`{length@0, bytes@8}`) — so each b-key must be rebuilt via
  `emit_materialize_string_from_bytes(keyAddr, KEY_LENGTH)` before the insert (the per-key cost, far below the
  geometric grow it replaces). Verified correct: fixture `merge-native-rt` (b-overwrites-on-TRUE / a-wins-on-
  FALSE-fallback / disjoint / empty-a / empty-b / **inputs unchanged = value semantics**; trueN=25 sum=19605
  k17=1700, falseN sum=11190 k17=17) + 3776 unit tests green. Commit: `<pending>`. Original analysis (kept):
  **MEASURED ~5ms of a P1 row; the earlier "base copy inherent → marginal" note was an UNMEASURED assumption
  and is WRONG.** Decomposed `mapchurn iterate` (14.4ms) this session by editing the benchmark's
  merge line (release `--run 10`, box-local): remove the whole merge line → **5.2ms** (so merge+its `keys(mg)`
  = 9.2ms, 64% of iterate); keep merge but drop `keys(mg)` → 12.4ms (merge alone = **7.2ms**); replace merge
  with a bare `MUT mg = m` owning copy → 7.26ms (base copy = **2.06ms**); so **the 10 inserts of `other`'s
  entries cost ~5.1ms** (~510ns each — ~10× too slow for an amortized in-place append). CONFIRMED not a
  FOR-EACH artifact: an inline plain-`FOR` doing 10 `mg = set(mg, toString(ki), ki)` into a copy of `m` is
  ALSO ~12.35ms. So **inserting new keys into a *copy of* a 1000-entry map is realloc/rehash-heavy** (churn's
  `m = set(m,…)` on a from-scratch map IS cheap/in-place, so this is specific to growing a tight copy — likely
  the tight copy has zero capacity headroom and/or each grow rebuilds the whole bucket index, `BUCKETS_READY`
  interplay). **Native merge fixes it by PRESIZING** result to `count(a)+count(b)` up front (one alloc, one
  bucket build) then bulk-inserting `b` with no further grow → merge ~7.2 → ~2-3ms → **iterate 14.4 → ~9-10ms**
  (~30-35%; stays P1 vs py 7.55 but a real win). **Edit points:** new `lower_collection_merge_call` in
  `builder_collection_queries.rs` (model on `lower_collection_map_values_call:3472` which already does
  `copy_collection_tight` + a per-entry loop): (1) copy `a` but RESERVED to `count(a)+count(b)` (need a
  copy-with-capacity or copy-tight-then-`reserve` map primitive — the missing piece; list has
  `reserve_integer_index_list`, maps need the analogue); (2) iterate `b`'s entry table (COUNT@8, stride 40,
  KEY/VALUE offsets like mapValues), read key+value; (3) if `!preferB` probe `hasKey(result,key)` (reuse
  `emit_collection_payload_matches_value_branch`) and skip on hit; else `lower_map_set_in_place(result, key,
  value)`; (4) dispatch gate `#collections_merge$K$V` in `builder_values.rs`. Checksum-verified, no UAF.
  **Also worth checking (higher leverage, may subsume D3):** WHY `x = set(x,…)` reallocs/rehashes per insert
  when `x` is a copy — CONFIRMED root cause: `copy_collection_tight` (`builder_collection_layout.rs:430`) always
  sizes the copy TIGHT (`capacity==count`, `dataCapacity==dataLength`, via
  `emit_write_list_header_from_registers:516`), so the first insert of a NEW key forces a geometric grow
  (entry+data realloc + bucket re-reserve). A plain owning copy MUST stay tight (value semantics — the caller
  may never insert), so this can't be fixed generally; it's merge-specific. **PRESIZE RECIPE (the missing
  ~100-line piece):** write a `copy_collection_with_capacity(a, extraEntries, extraData)` variant of
  `copy_collection_tight` — alloc `HEADER + (a.count+extraEntries)*ENTRY + (a.dataLength+extraData) +
  buckets(a.count+extraEntries)` (reuse the `emit_checked_size_*` + `emit_reserve_map_buckets` sizing), copy
  a's entries+data verbatim (the `emit_block_copy_advance` blocks at `:543`/`:565`), but store CAPACITY :=
  `a.count+extraEntries` and DATA_CAPACITY := `a.dataLength+extraData` EXPLICITLY (the tight header helper
  writes `capacity==count`; override the two fields, or thread capacity/dataCapacity params into a shared
  header write — `emit_write_collection_header:1512` is compile-time-count only, so it does NOT fit). For merge,
  `extraEntries = b.COUNT`, `extraData = b.DATA_LENGTH` (both loaded from b's header @8/@24). Then loop b's
  entries with `lower_map_set_in_place` (no grow now — presized), guarded by `hasKey` when `!preferB`. Verify
  with a targeted `artifact-gate <map-byte-identity-sel>` (not the full 18-min gate) + map checksums + a
  `merge` fixture (overlapping keys with preferB true/false, disjoint keys, empty a / empty b).

## Acceptance
map/mapchurn checksums (catch bucket corruption as a wrong lookup) + `cargo test` + map fixtures +
`scripts/artifact-gate.sh`.
