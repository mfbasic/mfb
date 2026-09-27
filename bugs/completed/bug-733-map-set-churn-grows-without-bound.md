# bug-733: a `Map`/`Set` whose size stays constant under in-place `set`/`removeKey` (`add`/`remove`) churn grows without bound

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: MEDIUM (memory; output is correct)
Class: Other (memory / scalability)

Status: FIXED (`800a1b966`, 2026-09-25) — history: **2026-09-25: fixed on `worktree-B-732` (Phases 1–2, verified; see
below). The full suite is still owed (Phase 3).**
Regression Test: `tests/runtime/rt_self_update_cost.rs`
(`a_churning_map_or_set_does_not_grow_without_bound`). Repro: `bugs/repro/bug-733-map-set-churn-grows-without-bound.mfb`

Found while fixing bug-732. A `MUT` map that keeps 64 keys — each round `set`s a new
key and `removeKey`s the oldest — holds 2.2 MB of arena at its peak after 16,000
rounds and 24.9 MB after 128,000. A `Set OF Integer` doing the same with `add` and
`remove` goes from 2.1 MB to 15.9 MB. Neither ever holds more than 65 entries. The
output is correct.

**The single correct behavior a fix produces:** a map or set's peak memory under
churn is bounded by a constant factor of its live entries — the same for 8× the
rounds — as a variable-width list's has been since bug-732.

References:

- `bugs/bug-732-self-update-arms-cost-collection-size.md` — the list half of the same
  problem, and the design this bug can reuse (reclaim dead bytes at every grow).
- `./mfb spec memory collections` (*Compaction*: "A `Map`/`Set`'s dead bytes are not
  reclaimed by any grow"; *Map Updates*).
- `.ai/collections.md`, "In-place map mutation: branch arg order, dead slack,
  BUCKETS_READY".

## STATUS: FIXED (`800a1b966`)

Landed on `main` by merging the combined `/fix-bug` integration branch; full suite green on the integration branch at `800a1b966` (bug-652/732/733/540/699/687/734 + main `f332f61a3`): `cargo test --release --no-fail-fast` — 242 test binaries, 6198 passed, 0 failed, 24 ignored, `artifact_gate_all` ok; `scripts/artifact-gate.sh target/release/mfb all` 0 diffs / 2160 goldens after regenerating the 72 `.ncode(sum)` goldens the fixes moved (`82f1a43bb`, `800a1b966`), classified against detached `main` (0 diffs / 2160 at both `4df37a4fe` and `f332f61a3`).

Re-verified after the bug-732/733 reclaim follow-up (`3a7232241`, goldens `596d1300e`): `cargo test --release --no-fail-fast` 242 binaries / 6200 passed / 0 failed / 24 ignored; `scripts/test-accept.sh` 1556 passed, 0 mismatches; `scripts/artifact-gate.sh all` 0 diffs / 2160.

## Failing Reproduction

```
CARGO_TARGET_DIR=/tmp/b732-target cargo build --release --bin mfb
mfb init /tmp/b733 && cp bugs/repro/bug-733-map-set-churn-grows-without-bound.mfb /tmp/b733/src/main.mfb
cd /tmp/b733 && mfb build --debug
./build/b733.out map 16000 2>&1 | grep peak_live_bytes
./build/b733.out map 128000 2>&1 | grep peak_live_bytes
```

Measured 2026-09-25 with the release `mfb` from `worktree-B-732`'s base
(`d92e9d129`) and with its bug-732 fixes applied — identical:

| shape | rounds | `arena.0.peak_live_bytes` |
| --- | --- | --- |
| `map` (`set` new key, `removeKey` oldest; 64 kept) | 16,000 | 2,186,208 |
| `map` | 128,000 | 24,899,088 |
| `set` (`add` new, `remove` oldest; 64 kept) | 16,000 | 2,099,760 |
| `set` | 128,000 | 15,943,344 |

- Observed: peak memory grows with the number of rounds (×11 and ×7.6 for ×8).
- Expected: the same peak at 16,000 and 128,000 rounds.

Contrast: the same churn on a `List OF String` (bug-732's
`a_churning_list_does_not_grow_without_bound`) stays flat at about 12.7 KB.

## Root Cause

An in-place removal (`lower_map_remove_key_in_place`, and `difference` /
`symmetricDifference` through `_mfb_rt_map_compact_from`) drops the entry but leaves
its key and value bytes in the data region as dead slack. Nothing ever reclaims
them:

- `lower_map_set_in_place` appends a new key+value at the data tail and, when the
  data region is full, grows it (`mapset_vgrow_dcap`, and the entry grow) by copying
  `dataLength` bytes verbatim, dead slack included, stepping the capacity from the
  old one;
- `emit_map_reserve` does the same for the batch forms;
- `copy_collection_tight` copies `dataLength` bytes verbatim too (so a copy carries
  the slack but does not accumulate it).

For a fixed-width key and value (`Set OF Integer`) the data capacity is tied to the
entry capacity (`emit_fixed_width_data_capacity`, bug-621), so the slack also forces
the entry table to grow: the whole block grows with the rounds.

This predates bug-732 (plan-86-D1 introduced the slack model, and
`.ai/collections.md` claimed the tight copy dropped it, which it does not).

## Goal

- The repro's peak is the same at 16,000 and 128,000 rounds for both shapes.
- No `set`/`removeKey`/`add`/`remove` loop gets slower: the reclaim is paid for by a
  grow, as bug-732's list reclaim is.

### Non-goals (must NOT change)

- Insertion order, the hash index's correctness, failure atomicity, and the
  per-call cost bug-732 established for `removeKey`/`remove`/`difference`/
  `symmetricDifference`.
- Do not "fix" it by compacting the data region on every removal: that makes each
  removal O(size) again (bug-732 finding 2).

## Blast Radius

- `lower_map_set_in_place` (both grow sites) and `emit_map_reserve` — the grows that
  copy slack verbatim; the fix belongs here.
- The record-field (`InlineGrow`) forms of the same grows.
- `copy_collection_tight` — carries slack, does not accumulate it; out of scope.

## Fix Design

(As filed — see *Corrections* for what shipped.) The list fix (bug-732, `emit_list_reclaim_before_grow`) is the model:
at a map grow, total the live key and value bytes (an O(count) walk the grow's entry
copy already costs) and, when the dead bytes outnumber them, repack the entries'
payloads into the new block in entry order, rewriting both offsets. A repack that
changes `dataCapacity` moves the bucket region, so it leaves `bucketsReady = 0`, as a
grow already does.

## Corrections

- **C1 — the reclaim must not allocate.** The first version (commit-free, measured
  2026-09-25) packed through an arena scratch block and copied back. It fixed the
  churn (every shape flat) but `rt_inplace_self_update`'s allocation bound failed
  51 of 231 map/set case/site pairs — `remove` +1,000 blocks per 2,000 runs,
  `symmetricDifference` +500, `removeKey` +333 — because a small map reclaims every
  few statements, which makes the scratch a per-statement allocation (the spec's
  "No per-statement allocation" rule). The shipped helper slides the payloads
  down in place instead. Doing that safely needs them visited in increasing
  offset: keys always are in entry order; a value moved to the tail by a growing
  `set` is not, so those values are heap-sorted by offset in the bucket region and
  merged in (the index is then rebuilt lazily). If the keys were ever out of order
  the helper does nothing.
- **C2 — out-of-order values are a churn shape of their own.** A 64-key
  `Map OF String TO String` whose values keep outgrowing their slots (no removals)
  grew 113,072 → 844,176 B at the base for 8× the rounds. It is in the test
  (`valueChurn`) and is flat after the fix.
- **C3 — the grow sites.** Besides the two `lower_map_set_in_place` grows, the
  batch reserve (`emit_map_reserve`, used by `union`, `symmetricDifference`,
  `merge`) reclaims first too. A record field that is not its record's last
  inlined field rebuilds by copying (`fieldCopy`), and was already flat at the base.

- **C4 — the reclaim trigger underflowed (found at integration, 2026-09-26;
  fixed in `3a7232241`, test `a3518ffba`; Status stays FIXED).** The helper
  decided with `dataLength - live > live`. `live` counts entry-order alignment
  pads; once moved values are packed last, a map needs fewer, so after a repack
  `dataLength < live`, the subtraction wrapped, and the helper kept repacking and
  returning 1 — the grow retried forever. `list-set-shared-offset-rt` (a
  `Map OF Integer TO String` whose values keep outgrowing their slots) hung at
  `2c228550a`. Now `dataLength > 2 * live`; the packed length is at most
  `2 * live`, so a repack is never followed by another.
  `a_map_reclaim_terminates_when_moved_values_pack_tighter` pins it (RED: did not
  finish in 30 s). No earlier churn shape combined 8-byte keys with relocated
  values. See bug-732 C6/C7 for the list half found by the same audit.

## Phases

### Phase 1 — failing test
- [x] A churn test: peak live bytes for 2,000 vs 16,000 rounds of the repro's two
      shapes, as `rt_self_update_cost`'s `a_churning_list_does_not_grow_without_bound`.
      → `a_churning_map_or_set_does_not_grow_without_bound`, eight shapes over a map
      or set held at 64 entries: `map` (String keys), `intMap` (fixed-width key and
      value), `set`, `valueGrow` (alternating value length; a guard), `valueChurn`
      (values outgrowing their slots), `setAlgebra` (`union` + `difference`),
      `fieldCopy` (non-last record field) and `fieldLast` (last record field). It
      fails if any shape's peak grows by more than a quarter.
      **RED at the base `d92e9d129`** and at this branch before the fix, identical:
      `CARGO_TARGET_DIR=/tmp/b732-target MFB_TEST_EXE=<base mfb> cargo test --test rt_self_update_cost -- --test-threads=1 --nocapture churning_map`
      → `map` 421,152 → 3,279,168 B, `intMap` 276,752 → 2,099,792, `set` 276,736 →
      2,099,776, `valueChurn` 113,072 → 844,176, `setAlgebra` 185,648 → 1,141,296,
      `fieldLast` 421,264 → 2,186,320; `valueGrow` and `fieldCopy` flat (20,848 and
      16,048). The documented reason: the fix below only adds the reclaim, and
      with it every shape is flat.
Commit: bade50f47 (filed), 32888428d (RED test)

### Phase 2 — fix
- [x] Reclaim dead key/value bytes at every map grow; update the spec *Compaction*
      and *Map Updates* sections and `.ai/collections.md`.
      → `_mfb_rt_map_reclaim` (`lower_map_reclaim_helper`, in place, allocates
      nothing, returns 0/1), called through `emit_map_reclaim_before_grow` at the
      entry grow and value grow of `lower_map_set_in_place` and at
      `emit_map_reserve`, each retrying its room check after a repack. No header
      change: the helper uses the bucket region (sort space) and a reserved entry
      byte (transient mark, cleared before return). Spec *Compaction*, *Map
      Updates* and the Self-updates "Cost" bullet; `.ai/collections.md` map notes
      (including C1). A new unit test (`inplace_compact`,
      `an_in_place_map_or_set_grow_reclaims_dead_bytes_first`) pins the call at
      `set`, `add` and `union`. No `mfb man` page states map memory behaviour, so
      no man change.

**GREEN:** the same command with this branch's release `mfb` → passed; every shape
flat: `map` 49,440 B, `intMap` 27,920, `set` 14,080, `valueGrow` 20,848,
`valueChurn` 27,024, `setAlgebra` 14,160, `fieldCopy` 16,048, `fieldLast` 49,552 at
both 2,000 and 16,000 rounds. `intMap` and `set` allocate 18 / 17 blocks in total
at both sizes (the reclaim allocates nothing). CPU stays linear: `map` 0.04 →
0.33 s, `set` 0.03 → 0.27 s, `valueChurn` 0.00 → 0.06 s user for 20,000 → 160,000
rounds. The repro: `map` 128,000 rounds peaks at the same bytes as 16,000.

Other checks (release `mfb` of this branch):
- `rt_self_update_cost` — all 11 pass, including the value program's new
  `mapReclaim` section (a `Map OF String TO Node` with graph-owning values, a
  `Set OF String` and a `Map OF Integer TO String` with outgrowing values, 600
  rounds at 8 entries; output identical to the base compiler's) and its `--debug`
  leak check (`alloc_calls = free_calls`, `live_bytes 0`, no double free).
- Three fuzz programs (`set`/`removeKey`/`getOr`/`hasKey` over Integer and String
  keys; `add`/`remove`/`difference`/`symmetricDifference` over Integer and String
  elements; `set` with growing values + `removeKey`, 3,000–60,000 rounds) print
  the same as the base compiler on macos-aarch64 and on windows-x86_64 (box 2230);
  the third, under `--debug`, frees every block. The Linux boxes were down, so
  x86-64 SysV and riscv64 were compiled only.
- `rt_inplace_self_update` filtered to the map/set lines
  (`MFB_SELF_UPDATE_FILTER='collections::removeKey|collections::remove(|collections::difference|collections::symmetricDifference|collections::union|collections::intersection|collections::merge|collections::set(|collections::add(|collections::mapValues'`)
  → 3 passed (the allocating first version failed 51 pairs here; see C1).
- `rt_inplace_failure_atomic`, `rt_recursive_value_collection_drops`,
  `rt_recursive_map_transfer`, `rt_imported_record_map_field_keys`,
  `rt_merge_fast_path_frees`, `rt_inplace_field_{mixed,loop,nested}`,
  `rt_inplace_global_record`, `rt_res_state_inplace_mutation`,
  `codegen_inplace_{record_field,remove_at_insert}`, `inplace_self_update_census`;
  `cargo test --bin mfb -- inplace_compact self_update map_compact spec
  citations_resolve` (66); `cargo check --all-targets` clean.
Commit: 8e260af26 (fix), 5c3804b48 (spec, .ai, doc)

### Phase 3 — full validation
- [x] Full suite.
      **Expected golden shifts** (measured on macos-aarch64, byte-identity covers
      built by this branch against bug-732's end state): **net, tcp, tls, udp**
      newly shift — each only gains the `_mfb_rt_map_reclaim` helper function and
      the call in the functions with an in-place map grow — on top of bug-732's
      set (audio, collections, csv, http, json, regex, strings, term,
      `syntax/app/app-mouse-surface`, `syntax/app/macos-app-mode-term`), where the
      map-growing functions also gain the call. The other native-golden fixtures are
      byte-identical. `.ast`/`.ir` cannot move.
Commit: 800a1b966 (goldens + full suite)
