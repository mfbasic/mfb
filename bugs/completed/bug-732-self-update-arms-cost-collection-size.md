# bug-732: six in-place self-updates cost the size of the collection, not the work they do, so build loops that use them are O(n²)

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: MEDIUM (performance; output is correct)
Class: Other (performance / scalability)

Status: FIXED (`800a1b966`, 2026-09-25) — history: **2026-09-25: all six findings fixed on `worktree-B-732` (Phases 1–2
landed, re-measured linear; see the table under Phase 2). The full suite is still
owed (Phase 3).** One pre-existing memory bug found on the way is filed as bug-733.
Regression Test: `tests/runtime/rt_self_update_cost.rs`. Repro: `bugs/repro/bug-732-self-update-arms-cost-collection-size.mfb`

Found by bug-652's Phase 1 audit of `SELF_UPDATE_TABLE`. bug-652 records the owner's
rule (2026-09-25): a `MUT` self-update `x = f(x, …)` must cost time proportional to
the work the call actually does — the elements it adds, removes or moves, or, for an
operation that must test every element, the elements it tests. It must never cost the
size of the part of the collection it leaves alone. An arm that accepts the statement
and still costs O(size) is a bug (`.ai/collections.md`, "In-place mutation").

Six in-place forms break the rule. Each one does O(1) work per call but walks all of
`x`, so a loop over it is quadratic. The output is always correct.

**The single correct behavior a fix produces:** each shape below scales linearly —
4x the iterations costs about 4x the time, the same as the `append` contrast.

References:

- `bugs/bug-652-take-drop-self-reassignment-copies-every-returned-element.md` — the
  rule, and the audit table these rows come from.
- `./mfb spec memory collections` (Self-updates, "Cost") — lists each of these as a
  current exception to the bound.
- `src/codegen/collection/assign/self_update.rs` — `SELF_UPDATE_TABLE`.

## STATUS: FIXED (`800a1b966`)

Landed on `main` by merging the combined `/fix-bug` integration branch; full suite green on the integration branch at `800a1b966` (bug-652/732/733/540/699/687/734 + main `f332f61a3`): `cargo test --release --no-fail-fast` — 242 test binaries, 6198 passed, 0 failed, 24 ignored, `artifact_gate_all` ok; `scripts/artifact-gate.sh target/release/mfb all` 0 diffs / 2160 goldens after regenerating the 72 `.ncode(sum)` goldens the fixes moved (`82f1a43bb`, `800a1b966`), classified against detached `main` (0 diffs / 2160 at both `4df37a4fe` and `f332f61a3`).

## Failing Reproduction

```
CARGO_TARGET_DIR=/tmp/b652-target cargo build --release --bin mfb
mfb init /tmp/b732 && cp bugs/repro/bug-732-self-update-arms-cost-collection-size.mfb /tmp/b732/src/main.mfb
cd /tmp/b732 && mfb build
/usr/bin/time -p ./build/b732.out <shape> <n>
```

Measured on `worktree-B-652` with the bug-652 fix applied, 2026-09-25. The table
gives `user` CPU seconds. The machine was heavily loaded, so wall time was not
usable.

| shape | loop body | n | user | 4n | user | ratio |
| --- | --- | --- | --- | --- | --- | --- |
| `append` (contrast) | `x = append(x, s)` | 10,000 | 0.00 s | 40,000 | 0.01 s | linear |
| `removeAtLast` | append 2, `x = removeAt(x, len(x) - 1)` on `List OF String` | 10,000 | 0.04 s | 40,000 | 0.61 s | ×15 |
| `removeKey` | `m = set(m, k, i)`, `set(m, "tmp", 0)`, `m = removeKey(m, "tmp")` | 10,000 | 0.65 s | 40,000 | 11.76 s | ×18 |
| `setRemove` | `s = add(s, i)`, `add(s, -1)`, `s = remove(s, -1)` | 10,000 | 0.77 s | 40,000 | 15.46 s | ×20 |
| `difference` | `s = add(s, i)`, `s = difference(s, {-5})` | 10,000 | 1.33 s | 40,000 | 23.67 s | ×18 |
| `symDiff` | `s = add(s, i)`, `s = symmetricDifference(s, {-5})` | 10,000 | 1.27 s | 40,000 | 22.41 s | ×18 |
| `stripSuffix` | `s = s & "x"`, `s = strings::stripSuffix(s, "q")` | 100,000 | 0.26 s | 400,000 | 3.95 s | ×15 |
| `popSorted` | `x = sort(x)` once, then `x = take(x, len(x) - 1)` until empty | 10,000 | 0.04 s | 40,000 | 0.72 s | ×18 |

- Observed: every shape except `append` costs about 16x for 4x the iterations.
- Expected: about 4x, like `append`.

Contrast: `popAppended` pops the same list without sorting it first. It is linear on
the bug-652 fix (0.00 s → 0.01 s). `take`/`drop` that remove nothing are O(1), which
bug-652 pins in `tests/runtime/rt_list_take_drop_in_place.rs`.

## Root Cause

Each finding was first reported by a read-only audit (bug-652 Phase 1). It was then
spot-checked by grepping the named loop labels, and confirmed by the timings above.

1. **`removeAt`, variable-width list.** `lower_list_remove_at_in_place` calls
   `emit_offset_compaction_fixup(…, "remove_inplace_fix")`
   (`src/codegen/collection/buffer/collection_buffer.rs`). That loop visits every
   surviving entry to fix its offset, whatever the index. It also moves every data
   byte after the hole in *data* order, which is not entry order. So removing the
   last element costs O(count). The fixed-width (`kind2`) path moves only the suffix.
2. **`removeKey` (Map) and `remove` (Set).** `lower_map_remove_key_in_place`
   (`src/codegen/collection/map/map_mutate.rs`) makes three passes:
   - `mrk_scan_loop` searches the key linearly, even when the key type is
     probe-eligible.
   - `mrk_shift_loop` shifts every later entry.
   - It stores `COLLECTION_OFFSET_BUCKETS_READY = 0`, so the next probe rebuilds the
     whole bucket index.
3. **`difference`.** `try_inplace_filter_set`
   (`src/codegen/collection/assign/builder_inplace_setmap.rs`) runs
   `emit_membership_marks` over all of `s`, then `lower_map_compact_in_place`
   (`mcompact_loop`), which resets the bucket index. Removing a one-element `t` costs
   O(|s|).
4. **`symmetricDifference`.** `try_inplace_symmetric_difference_assign` marks all of
   `s` (`inplace_symdiff_s`) and compacts it, even though at most |t| elements change.
5. **`String` window family.** `try_inplace_string_window_assign`
   (`src/codegen/collection/assign/string_self_update.rs`) always copies the whole
   result window to the start of the block (`inplace_str_window_move`). It does so
   even when the window already starts there. So `stripSuffix` and a no-match
   `stripPrefix` copy every kept byte onto itself. `trimEnd` also scans the string
   forward before trimming (`strings_trim_end_loop`, `gen_trim.rs`). Lesser cases
   (`mid`/`left` from 0, `trim` with nothing leading) pay the same self-copy.
6. **`take`/`drop`/`mid` truncation of an out-of-order list.** bug-652's fast path
   (`emit_compact_range_truncate`, `src/codegen/collection/list/list_compact.rs`)
   retracts `dataLength` only when the removed payloads are the tail of the data
   region. After `sort`, `prepend`, `insert` or a relocating `set` they are not.
   The call then falls through to the general compaction, which walks every entry
   to count live bytes for its dead-byte repack trigger. A list whose payloads stay
   out of order pays O(count) on every pop. The live-byte total is not stored
   anywhere, so it can only be recomputed by that walk.

## Goal

- Every shape in the table scales linearly (4x iterations ≈ 4x CPU time).
- `SELF_UPDATE_TABLE`'s `Arm` rows all meet the cost rule. The "Cost" exception list
  in `./mfb spec memory collections` becomes empty and is deleted.

### Non-goals (must NOT change)

- Value semantics, failure atomicity, the drop obligations of removed graph-owning
  elements, and the allocation bounds pinned by `tests/runtime/rt_inplace_self_update.rs`.
- Map/Set iteration order (insertion order).
- Do not "fix" a row by making its arm decline. Per the rule, a declining arm is
  correct only where a copy is required.

## Blast Radius

The audit table in bug-652 covers every `Arm` row. Rows not listed here met the rule
when measured or read. Note (not an offender under the rule): `set`/`add`/`union`/
`merge` scan linearly (`mapset_loop`) for a key type that is not probe-eligible
(`map_key_probe_eligible`). That cost belongs to the data structure's lookup, not to
the arm.

## Fix Design

(As filed — see *Corrections* for what shipped.) Each finding is independent and
can be its own phase:

- Findings 1 and 6 need either dead-byte accounting or a cheaper order/tail test.
  Adding a header field would change the collection layout, which bug-652 rejected
  for its own fix. Weigh that first.
- Findings 2–4 need a removal that probes the key, and bucket maintenance that does
  not rebuild the whole index.
- Finding 5 needs a skip when the window already starts at the block start, and a
  backward scan for `trimEnd`.

## Corrections

Every mechanism in *Root Cause* was confirmed by reading the lowering and by
the RED runs below; none was a false premise. What the work added:

- **C1 — findings 1 and 6 needed no header field (2026-09-25).** The doc asked to
  weigh dead-byte accounting in the header first. It was not needed: the O(count)
  live-byte walk moved from the removals to the **grows**, whose entry copy already
  costs O(count). Removals now leave a non-tail payload as dead bytes;
  `emit_list_reclaim_before_grow` runs at the realloc label of every in-place
  variable-width grow (append, bulk append, insert/prepend, and the record-field
  forms) and repacks (`emit_repack_list_data`, then retries the room check) when
  dead bytes outnumber live ones. That keeps the data capacity within a constant
  factor of the live bytes, which is what bug-652 C1 needed. The collection layout
  is unchanged.
- **C2 — a copying grow must reclaim too.** The churn test showed a hole in C1:
  at a record field that is not its record's last inlined field, `removeAt` runs
  in place but `append` rebuilds through `lower_list_insert_collection`, which
  copied the source's data region verbatim. A remove/append loop there grew
  without bound and copied ever more per call: peak live 75,552 B at 2,000 rounds,
  537,568 B at 16,000, 4,233,568 B at 128,000 (the pre-fix compiler: 9,648 B flat,
  because its `removeAt` compacted). `lower_list_insert_collection` now packs the
  source's live payloads when its dead bytes outnumber them.
- **C3 — the bucket index can be repaired after a delete.** `.ai/collections.md`
  said an in-place delete must set `BUCKETS_READY = 0` because open addressing
  without a tombstone cannot be repaired. Linear probing is repaired by
  backward-shift deletion (`_mfb_rt_map_bucket_delete`); the entries after the
  removed one are renumbered as they slide down (`_mfb_rt_map_compact_from`).
  The same note claimed `copy_collection_tight` drops dead map slack; it copies
  `dataLength` verbatim. Both corrected.
- **C4 — two more O(|s|) walks than the doc listed.** `difference` also sized its
  `String` scratch from the longest key of *s* (`emit_borrow_area_size(dest, …)`),
  and `symmetricDifference` from the longest of *s* and *t*. Both now size from
  *t* only. The old `symmetricDifference` also compacted *s* before its reserve,
  so an `ErrOutOfMemory` from the reserve left *s* already shrunk; the reserve now
  runs first.
- **C5 — found on the way.** A `Map`/`Set` whose size stays constant under
  `set`/`removeKey` churn grows without bound (dead key/value slack is never
  reclaimed): filed as **bug-733**, pre-existing at the base. `mfb man collections`
  `removeAt`, `insert` and `replace` said "there is no cheap in-place shape … every
  call copies the list", false since plan-121-B/plan-142: corrected.

- **C6 / C7 — two regressions found at integration (2026-09-26), fixed in
  `3a7232241` (tests `a3518ffba`); Status stays FIXED.** Acceptance fixture
  `rt-behavior/collections/list-set-shared-offset-rt` spun >120 s CPU at
  `2c228550a` (0.21 s on main). Bisected across this bug's commits with the
  release compilers kept per step: the bug-732 end state (`/tmp/b732-cur2-mfb`)
  ran it in time, the first bug-733 build hung — the fixture's
  `Map OF Integer TO String` whose values keep outgrowing their slots.
  - **C6 (bug-733's reclaim):** the trigger `dataLength - live > live`
    underflowed. `live` counts entry-order alignment pads; a packed map with its
    moved values last needs fewer, so right after a repack `dataLength < live`,
    the subtraction wrapped, and the helper repacked and returned 1 forever. Now
    `dataLength > 2 * live`, which a packed map can never satisfy.
  - **C7 (this bug's list reclaim), found by the audit the coordinator asked for:**
    a repack keeps `capacity` and sizes the data from the live bytes, a verbatim
    grow steps `capacity` and `dataCapacity`. A list holding a few elements whose
    payloads keep changing length alternated the two and doubled its entry
    capacity each cycle — exponential memory: a two-`String` list under `set` +
    `append` + `removeAt(0)` peaked at 933,568 B after 1,200 rounds (1,088 B after
    150), a record list at 3.1 GB, and an `insert` shape never finished. The list
    reclaim now also repacks on any dead bytes while `count < capacity`. The
    shape exists at the base `d92e9d129` too for `set` + `prepend` + `take`
    (bug-652's out-of-order compaction repacks the same way): 20 rounds took
    10 s there.
  - **Why no earlier test caught them:** the churn tests used `String` keys
    (alignment 1, so pads never differed) or values that only grew in place, and
    their lists held 64 elements of fixed-length payloads, where the repack /
    grow alternation never set in within 16,000 rounds. The new tests use 8-byte
    keys with relocated values, and two-element lists with lengths that rise and
    fall. The audit also ran a 30,000-round fuzz over `Map OF Integer TO String`,
    `Map OF String TO Rec`, `Map OF Float TO Integer`, `Map OF Integer TO Rec` and
    record/`String` lists with every mutation: output identical to the base where
    the base finishes (the base runs out of memory on the 12,000-round list part),
    `--debug` frees every block, and CPU fell from 2.24 s to 0.16 s against
    `2c228550a`.
  - Map grows keep their `dataCapacity` across the in-place reclaim, so they do
    not have C7's alternation; the churn test stays flat.

## Phases

### Phase 1 — failing tests
- [x] A scaling test per finding (CPU-time ratio, as `tests/runtime/rt_list_take_drop_in_place.rs`).
      → `tests/runtime/rt_self_update_cost.rs`: one test per finding, each timing
      `ROUNDS = 8` runs of `n` against one run of `8n` (child CPU via `wait4`;
      bound ×3 + 0.05 s) and checking each run's result line; an `append` guard;
      a value-semantics program (results, iteration order, every surviving key
      probed back after removals, aliases and copies untouched; checked by hand and
      identical to the pre-fix compiler's output); its `--debug` build freeing
      every block (`alloc_calls = free_calls`, `live_bytes 0`,
      `double_free_skips 0`); and a churn bound (peak live bytes of a 64-element
      list that keeps losing non-tail payloads, 2,000 vs 16,000 rounds, in a local,
      a last record field, a global, and a non-last record field).
      **RED at the base `d92e9d129`** (release `mfb` from that tree, copied to
      `/tmp/b732-base-mfb`):
      `CARGO_TARGET_DIR=/tmp/b732-target MFB_TEST_EXE=/tmp/b732-base-mfb cargo test --test rt_self_update_cost -- --test-threads=1`
      → 7 failed, 3 passed. The failures: `removeAtLast` ×7.49, `removeKey` ×7.96,
      `difference` ×7.74, `symDiff` ×7.78, `strip` ×7.76, `popSorted` ×7.61, plus
      the churn test's first version (see below). Each test stops at its first
      shape; the second shapes, run at the base with `/tmp/b732s/cal.sh`:
      `removeAtSorted` 0.06 → 0.50 s, `setRemove` 0.20 → 1.60 s, `trim`
      (n = 25,000) 11.75 → 92.34 s, `popInserted` 0.10 → 0.87 s for 8 runs of n
      vs one of 8n. The value, leak and `append` tests are green at the base.
      The churn test's first draft measured a `sortPop` shape, which the base
      itself fails (its retained strings change length): replaced by shapes whose
      base peaks are flat. The final churn test is green at the base and was
      proved load-bearing against the fix with its reclaim disabled (see C1/C2).
Commit: 4cf153c2c

### Phase 2 — fixes
- [x] Finding 5 — `try_inplace_string_window_assign` skips the move when the
      window already starts at the block start; `lower_strings_trim_window` trims
      trailing whitespace by stepping back from the end (`emit_trim_end_backward`,
      the same whitespace set as `emit_unicode_whitespace_branch`).
- [x] Finding 2 — `emit_map_find_index` (hash probe for a probe-eligible key type,
      entry scan otherwise), `emit_map_mark_removed` (`_mfb_rt_map_bucket_delete`,
      backward-shift deletion, then `COLLECTION_ENTRY_FLAG_REMOVED`) and
      `emit_map_compact_from` (`_mfb_rt_map_compact_from`, slide + renumber); the
      index stays built. Three codegen tests detected the in-place `removeKey` by
      its `mrk_scan(_loop)` label, which an `Integer` key no longer emits; only
      those assertion lines now look for `mrk_done` (the four questions answered in
      the commit message).
- [x] Findings 3, 4 — `difference` claims `t`'s hits in `s` and compacts once from
      the first; `symmetricDifference` reserves for `t`, claims the hits, marks the
      misses, compacts, adds the misses. `intersection` keeps its mark pass. The
      unit test that required `difference` to mark every entry of `s` now requires
      the claim loop and the absence of that pass.
- [x] Finding 1 — `removeAt` shifts only the later entries; the removed payload is
      retracted over (tail) or left as dead bytes; `emit_list_reclaim_before_grow`
      at every in-place grow (C1).
- [x] Finding 6 — `emit_compact_range_truncate` leaves a non-tail removed payload
      as dead bytes and keeps walking instead of falling to the general path.
- [x] C2 — the copying insert packs a list with more dead than live bytes; its
      fixed-width form stays byte-identical (the first version reloaded three
      operands unconditionally, caught by the compress/crypto covers shifting
      with no bug-732 marker, and fixed).
- [x] Spec: `memory/05_collections.md` — the "Cost" exception list is gone (the
      bullet states each form's cost), plus *Map Hash Index*, *Compaction*,
      `dataLength`, entry `flags`, the window family and the value-path `insert`;
      `language/12_collections.md` drops its pointer to the list.
      `.ai/collections.md` — the shrink-cost note, a "Dead bytes" rule, the
      aliasing table's `removeAt` row, the map notes (C3, bug-733).
      Verified: `cargo test --bin mfb spec` → 44 passed; `citations_resolve` ok;
      `MFB=… scripts/spec-census.sh --citations` → `MISS-SYMBOL 0`;
      `mfb spec memory collections` / `language collections` render with 0 `[[`.
- [x] Man: `removeAt`, `insert`, `replace`, `take` (C5).
      `MFB=… scripts/man-census.sh --memory-scope` → 0 unclassified;
      `MFB=… scripts/man-run-examples.sh collections --run` → 140 built, 140 ran,
      0 failed.

**GREEN** (release `mfb` built from this branch):
`CARGO_TARGET_DIR=/tmp/b732-target MFB_TEST_EXE=<that mfb> cargo test --test rt_self_update_cost -- --test-threads=1 --nocapture`
→ 10 passed. Ratios for the same work: `removeAtLast` ×1.46, `removeAtSorted`
×1.40, `removeKey` ×1.88, `setRemove` ×1.41, `difference` ×1.48, `symDiff` ×1.79,
`strip` ×1.10, `trim` ×1.01, `popSorted` ×1.43, `popInserted` ×1.67 (all at
0.001–0.015 s CPU, where the ratio is timer noise), `append` ×1.50. Churn peaks
flat: 12,688 / 12,688 / 12,944 / 12,624 / 10,928 B at 2,000 and 16,000 rounds.

Re-measured with the repro (`/usr/bin/time -p`, `user` s; the base binary is the
same program built by the base compiler; load average 50–80):

| shape | n | before: n → 4n | after: n → 4n |
| --- | --- | --- | --- |
| `append` (contrast) | 10,000 | 0.00 → 0.01 | 0.00 → 0.01 |
| `removeAtLast` | 10,000 | 0.04 → 0.58 (×15) | 0.00 → 0.01 |
| `removeKey` | 10,000 | 0.62 → 11.50 (×19) | 0.00 → 0.03 |
| `setRemove` | 10,000 | 0.76 → 14.37 (×19) | 0.00 → 0.02 |
| `difference` | 10,000 | 1.34 → 23.44 (×17) | 0.00 → 0.01 |
| `symDiff` | 10,000 | 1.25 → 22.16 (×18) | 0.00 → 0.01 |
| `stripSuffix` | 100,000 | 0.22 → 4.14 (×19) | 0.00 → 0.01 |
| `popSorted` | 10,000 | 0.04 → 0.70 (×18) | 0.00 → 0.01 |

Other checks (all against the release `mfb` of this branch):
- `cargo test --bin mfb -- inplace_compact self_update compact_in_place map_compact string_self_update trim remove_at`
  → 19 passed (incl. `every_arm_row_fires_at_every_enabled_site`); `inplace_compact`
  → 10 passed with the four new label tests.
- `rt_list_take_drop_in_place` (5), `rt_list_append_growth_bounds` (8),
  `rt_list_set_widening_linear`, `rt_inplace_failure_atomic`,
  `rt_recursive_value_collection_drops`, `rt_inplace_field_{loop,mixed,nested}`,
  `rt_inplace_global_record`, `rt_for_each_self_update`, `rt_global_self_update`,
  `rt_inplace_item_operand`, `rt_inplace_sort`, `rt_inplace_append_builtin_call`,
  `rt_reduce_collection_accumulator_frees`, `rt_imported_record_map_field_keys`,
  `rt_operand_snapshot`, `rt_recursive_map_transfer`, `inplace_self_update_census`,
  `codegen_inplace_{append_call_result,record_field,remove_at_insert}`,
  `codegen_string_set_shift`, `rt_res_state_inplace_mutation` (24),
  `rt_lambda_capture_self_update` — all pass.
- A 50,000-operation fuzz of `set`/`removeKey`/`getOr`/`hasKey` (Integer and String
  keys) and `add`/`remove`/`difference`/`symmetricDifference` (Integer and String
  elements) prints the same as the pre-fix compiler on macos-aarch64, and on
  windows-x86_64 (box 2230); the value program matches there too. The Linux boxes were down (connection refused), so
  x86-64 SysV and riscv64 were only compiled, not run.
- The `rt-behavior` string fixtures `self-update-window-valid`,
  `self-update-grow-valid`, `strings-empty-needle-rt`,
  `strings-artifact-coverage-rt`, `func_typename_builtin_calls` and
  `astrings/tier-b-transforms-rt` match their golden run output.
Commit: 3bc7e42d2, 54db20195, e255cdf35, 472dfe4b9, 0d9548bec, fcb06fe1d, 880ea7499, 33f8681f1, 17657bdb9, 601da4ab0

### Phase 3 — full validation
- [x] Full suite.
      **Expected golden shifts** (measured on macos-aarch64 by building every
      `tests/byte-identity/*` cover and every other fixture with a native golden,
      base vs this branch; every changed function carries a bug-732 marker
      label — `list_reclaim_walk`, `list_packed_walk`, `strings_trim_end_back*`,
      `inplace_str_window_in_place`, …): the byte-identity covers **audio,
      collections, csv, http, json, regex, strings, term** (their `.ncodesum` on
      all five targets), and **`syntax/app/app-mouse-surface`** and
      **`syntax/app/macos-app-mode-term`** (their app `.ncode`/`.ncodesum`). The
      other 20 covers and 9 native-golden fixtures are byte-identical, including
      compress and crypto (fixed-width lists only). `.ast`/`.ir` goldens cannot
      move (codegen only). Any other diff is a bug-hunt trigger.
      `rt_inplace_self_update` was run only on the lines this bug touches (the
      full matrix took bug-652 over 2h50m under this load):
      `MFB_SELF_UPDATE_FILTER='collections::removeAt|collections::removeKey|collections::remove(|collections::difference|collections::symmetricDifference|collections::take|collections::drop|collections::mid|collections::append|collections::insert|collections::prepend|collections::intersection|collections::union|collections::set(|collections::add(|strings::stripSuffix|strings::stripPrefix|strings::trim|strings::left|strings::right|strings::mid(value AS String' … cargo test --test rt_inplace_self_update`
      → 3 passed (1,359 s; the field-kind test runs every field kind). The rest of
      the matrix is left to the full suite. `cargo check --all-targets` is clean.
Commit: 82f1a43bb, 800a1b966 (goldens + full suite)

## Summary

bug-652 fixed the take/drop build loop and made the cost rule explicit. Its audit
found five more in-place forms, plus bug-652's own out-of-order truncation case,
that still walk the whole collection. All six are measured quadratic above. The
spec documents them as current exceptions until this bug lands.
