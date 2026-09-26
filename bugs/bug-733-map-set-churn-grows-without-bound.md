# bug-733: a `Map`/`Set` whose size stays constant under in-place `set`/`removeKey` (`add`/`remove`) churn grows without bound

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: MEDIUM (memory; output is correct)
Class: Other (memory / scalability)

Status: Open
Regression Test: none yet — see Phase 1. Repro: `bugs/repro/bug-733-map-set-churn-grows-without-bound.mfb`

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

Not designed. The list fix (bug-732, `emit_list_reclaim_before_grow`) is the model:
at a map grow, total the live key and value bytes (an O(count) walk the grow's entry
copy already costs) and, when the dead bytes outnumber them, repack the entries'
payloads into the new block in entry order, rewriting both offsets. A repack that
changes `dataCapacity` moves the bucket region, so it leaves `bucketsReady = 0`, as a
grow already does.

## Phases

### Phase 1 — failing test
- [ ] A churn test: peak live bytes for 2,000 vs 16,000 rounds of the repro's two
      shapes, as `rt_self_update_cost`'s `a_churning_list_does_not_grow_without_bound`.
Commit: —

### Phase 2 — fix
- [ ] Reclaim dead key/value bytes at every map grow; update the spec *Compaction*
      and *Map Updates* sections and `.ai/collections.md`.
Commit: —

### Phase 3 — full validation
- [ ] Full suite.
Commit: —
