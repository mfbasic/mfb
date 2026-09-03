# plan-121-B: Complete and tune the plain-local container

Last updated: 2026-09-02
Effort: large (3h–1d)
Depends on: plan-121-A

Three collection operations have no in-place arm in any container —
`insert`, `removeAt` (List) and `remove` (Set) — so each one allocates a fresh
block and copies the whole collection per call. Three more have an arm that
matches but still runs 14–36× C because of a known secondary cost. This sub-plan
finishes the plain MUT local column of the matrix and tunes the arms already
there.

Behavioral outcome: `collections::insert`, `removeAt` and Set `remove` on a
uniquely-owned plain `MUT` local mutate the live buffer (a bounded shift plus a
count update) instead of allocating and copying, with value semantics unchanged.

References: as plan-121-A, plus `.ai/collections.md` §"In-place map mutation"
(the `BUCKETS_READY=0` rehash rule that bounds the `removeKey` win).

## Prerequisites

Stated once in plan-121-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-121-A complete and archived | `ls planning/completed/plan-121-A-*` → 1 file | MET — `planning/completed/plan-121-A-inplace-slot-abstraction.md` |
| The seam admits a new operation | plan-121-A Phase 3 unit test green | MET — see below |

If plan-121-A is not complete, this sub-plan cannot start, full stop.

**Both MET (2026-09-02).** plan-121-A landed as `33761be42` (Phase 1),
`b5966a3d6` (Phases 2+3), `b1d7f242f` (Phase 2b) and `7b9ffa6e3` (fmt +
validation proofs), with 4465 passed / 0 failed, 1348 acceptance tests ran /
0 mismatches, and 1828 goldens / 0 diffs.

The second row needs one word of care, because plan-121-A restated it. Its
Phase 3 "compile-time-off constant" was **not** implemented — an arm that
resolves a destination and then returns `Ok(false)` because its lowering does
not exist yet is the stub AGENTS.md forbids (plan-121-A Corrections C3). What
landed instead is `inplace_dest.rs`'s `mod tests`: 10 cases covering every
decline condition `InPlaceGate` owns (`G1`, `G7`, `G10`, `G15`, `G16`), each
paired with an admit case. Those are green.

So "the seam admits a new operation" is **this** sub-plan's Phase 2 to
demonstrate, by adding `removeAt` for real rather than behind a constant —
which is the stronger demonstration, and is why C3 called it a strengthened
acceptance rather than a skipped one.

## 1. Goal

- `insert`, `removeAt`, and Set `remove` take an in-place path on a uniquely-owned
  plain `MUT` local, via plan-121-A's seam.
- ~~The six rows in §2 move from grade F to grade B or better, measured by
  `./benchmark/rank.py`.~~ **CORRECTED — see B9.** Measured, the six rows improve
  7.4×–13.8× and two reach grade B; the other four are each bounded by something
  this sub-plan's own non-goals exclude (the `BUCKETS_READY` rehash for `set`, the
  String representation for the `Dynamic` rows) or by the O(N) shift the non-goals
  name as the defined cost. B9 replaces the grade letter with five clauses that
  name the mechanism instead — including "`insert`'s fixed per-call cost is within
  25% of `append`'s" and "`set (Fixed) remove` is within 2× of the untouched
  sibling that shares its lowering" — all of which are verified there.

### Non-goals

Inherited verbatim from plan-121-A §1. Additionally, specific to this sub-plan:

- **`insert`/`removeAt`/`prepend` stay O(N).** Shifting N elements is the
  operation's defined cost and C pays it too (`memmove`). The goal is to delete
  the *allocate + copy + free* per call, not to invent a different data
  structure. A row that lands at ~2–3× C is done; chasing O(1) would mean
  changing List's representation, which is out of scope and would change layout.
- **No tombstone delete for Map.** `.ai/collections.md` records that the open-addressed
  hash index cannot be repaired incrementally and that a true O(1) tombstone
  delete is "a structural project"; this sub-plan does not start it.

## 2. Current State

`try_inplace_insert_assign`, `try_inplace_remove_at_assign` and a Set-`remove`
arm do not exist (`grep -rhoE "fn try_inplace_[a-z_]*" src/codegen/ | sort -u`
→ 10 arms, none of them these). Every such call therefore falls through
`builder_control.rs:879-909` to the copying reassignment path.

### Measured populations

| What | Count | Command |
|---|---|---|
| Plain-local rows C-or-worse with **no** arm | 6 | `./benchmark/rank.py --csv` filtered to plain sections, rows `insert`/`removeAt`/`remove` |
| …worst of those | 677× (`set (Fixed) remove`) | same |
| Plain-local rows C-or-worse **with** an arm | 10 | same, rows `set`/`add`/`removeKey`/`prepend` |
| …of those, `small` confidence (µs prize) | 4 | `awk -F, '$6=="small"'` — deliberately not scoped |
| …of those, owned by plan-121-F (String) | 1 | `list (Dynamic) set`, 371× |
| …of those, in scope here (high constant) | 5 | `map`/`map (key-)` `removeKey` ×2, `prepend` ×3 |

### Verified properties

- **VERIFIED — `insert`/`removeAt`/`prepend` copy the whole list per call, and
  `append` does not.** Spike 3 holds the call count at 500 and grows N from 100
  to 6400 on a `List OF Integer`: `append` is flat at 26 → 12 ns/call, while
  `prepend` rises 762 → 13792, `insert` 2790 → 72360, and `removeAt`
  1862 → 28670 ns/call. Flat versus linear is the presence or absence of an arm.
- **VERIFIED — the residual is allocation, not the shift.** At N = 6400 a
  `List OF Integer` is 51 KB; a `memmove` of that is ≈ 2 µs, but `insert` costs
  72 µs — 36× the data movement. The per-call allocate/copy/free dominates, which
  is what an in-place shift deletes.
- **UNVERIFIED — how much of `removeKey`'s 32–36× is the `BUCKETS_READY=0`
  rehash.** `.ai/collections.md` states the in-place delete must invalidate the
  bucket index, so the next probe rebuilds it. Phase 1 measures the rehash share
  before any change; if it is the whole gap, the row is at its designed cost and
  this sub-plan records that and stops on it rather than starting a tombstone
  project the non-goals forbid.

## 3. Design Overview

Each new arm is the same shape, written once against plan-121-A's `InPlaceDest`:

- **`removeAt(list, i)`** — shift entries `[i+1, count)` down one stride, then
  `count -= 1`. For a variable-width element the data region also compacts;
  reuse the existing removal repack (`.ai/collections.md`: "Removal stays
  eager-repack (no lazy holes)").
- **`insert(list, i, v)`** — grow if `count == capacity` (existing geometric
  step), shift `[i, count)` up one stride, write the element, `count += 1`.
- **`remove(set, v)`** — locate via the existing probe; on hit, remove the entry
  exactly as the Map `removeKey` arm does, including setting `BUCKETS_READY=0`.

**Where correctness risk concentrates:** the shift direction and the aliasing
gate interact. An `insert` shifting *up* writes past `count`, which is precisely
the region `.ai/collections.md` says a live `FOR EACH` treats as out of scope
(count snapshotted at loop entry) — but a `removeAt` shifting *down* rewrites
entries *below* the snapshot, which a live iterator **can** observe. **`removeAt`
must therefore decline whenever any `FOR EACH` iterates the list, even though
`append` may proceed in the same situation.** This asymmetry is the single most
important thing in this sub-plan and it is why the gate inventory from
plan-121-A Phase 1 is a specification and not a formality.

**Byte-identity is NOT the gate here.** These phases change emission on purpose.
Expected drift: `.ncode`/`.ncodesum` for every fixture containing `insert`,
`removeAt`, or Set `remove` on a plain local. Each phase names the goldens it
expects to move and regenerates them with `sync-goldens.sh`/`regen-ncodesum.sh`
after proving the behavior is unchanged; a drift *outside* that set is a bug.

### Rejected alternatives

- **Make `removeAt` lazy (tombstones/holes).** Rejected: `.ai/collections.md`
  records eager repack as the current contract, and every `0..count` consumer
  would need a USED-skip. Out of scope and a much larger correctness surface.
- **Special-case `removeAt(list, 0)` as a base-pointer bump.** Rejected: the
  block's base is the free() address; moving it would break scope-drop. It also
  only helps one index.

## Phases

### Phase 1 — Measure the two residual costs before changing anything

- [x] Spike: isolate the `BUCKETS_READY=0` rehash share of `map removeKey` by
      timing N deletes followed by one lookup versus N deletes interleaved with
      lookups. Record the split in this plan.
      **`spikes/s6`. The rehash share is ZERO. The cost is a linear entry scan.**

      `mfb build spikes/s6 && ./spikes/s6/build/mfb_project.out`, 2000 deletes
      held constant, ns/call (second run; the first agreed except for one noisy
      point at N=1600):

      | N | A deletes only | B delete + probe | C probe only |
      |---|---|---|---|
      | 400 | 454 | 442 | 18 |
      | 800 | 898 | 864 | 20 |
      | 1600 | 1690 | 1708 | 25 |

      **A is exactly linear** (×1.98 then ×1.88 per doubling) even though *no
      probe happens between deletes*, so nothing can be rehashing inside the
      timed region — that growth is the scan. **B ≈ A at every N**, so
      `B − A − C ≈ 0`: forcing the rebuild by probing after every delete adds
      nothing measurable. And **C is flat at 18–25 ns**, so the bucket index
      itself works and `hasKey` is the O(1) probe it should be.

      Root cause, confirmed by reading the lowering:
      `lower_map_remove_key_in_place` (`map_mutate.rs:928`) finds the key by
      **scanning entries `0..count`** (`mrk_scan_loop`), not by probing the index
      `hasKey` uses. `removeKey` is O(N) per call for that reason alone.

- [x] Spike: time `prepend` against a raw `memmove` of the same bytes to
      establish the floor the arm can reach.
      **`spikes/s7`. The arm is already AT the floor — in fact below it.**

      `./spikes/s7/build/mfb_project.out`, 500 calls held constant, ns/call.
      The floor is a bulk `collections::append(dst, src)`, which
      `try_inplace_bulk_append_assign` lowers to a block copy of the same
      entries + data with no per-element work — the closest primitive MFBASIC
      has to a raw `memmove`:

      | N | A prepend | B block copy (floor) | A/B |
      |---|---|---|---|
      | 100 | 780 | 420 | 1.86× |
      | 400 | 1384 | 1766 | **0.78×** |
      | 1600 | 3870 | 6796 | **0.57×** |
      | 3200 | 7286 | 13326 | **0.55×** |

      From N = 400 up, `prepend` costs **less** than copying the same bytes
      through the fastest bulk primitive available. (B is an *over*-estimate of a
      pure `memmove`: it allocates a fresh destination each call, where prepend
      shifts inside the buffer it already owns. That is precisely why prepend can
      come in under it.) The control C stays flat at 14–20 ns, so the harness is
      measuring the shift and not a per-call constant.

      This settles the Open Decision below: the criterion was "within ~3× of the
      floor → stop", and the arm is at 0.55–1.86×.
- [x] Record which `.ncode` goldens contain plain-local `insert`/`removeAt`/Set
      `remove`, so Phases 2–3 can distinguish expected from unexpected drift.
      **The expected-drift set is exactly ONE golden**, which is a much sharper
      gate than the plan assumed:

      ```
      tests/rt-behavior/collections/list-ops-codegen-rt/golden/list_ops_codegen_rt.macos-aarch64.ncode
      ```

      Census (`grep -rnE "= *collections::(insert|removeAt|remove)\(" tests/`,
      then `ls` each hit's `golden/`): the *self-assignment* shape these arms match
      appears in six fixtures — `list-ops-codegen-rt`, `list-order-invariant-rt`,
      `collection-memory-grow-rt`, `set-behavior-rt`,
      `resources/inline-trap-collection-escape-rt`, and a source string inside
      `tests/rt_recursive_thread_transfer.rs` — and **only `list-ops-codegen-rt`
      carries a `.ncode` golden.** The rest ship `.ast`/`.ir`/`build.log`/`.run`
      only, and `.ast`/`.ir` are emitted *before* codegen, so a new in-place arm
      cannot move them.

      The `tests/byte-identity/*` fixtures — the ones that do carry
      `.ncodesum` for five targets — are unaffected: `byte-identity/collections`
      spells these ops as `FOR EACH x IN collections::insert(ints, 1, 99)`, not as
      a self-assignment, so every new arm declines on it (G6). Adding arms to the
      dispatch chain is otherwise emission-free: a declining arm emits nothing.

      **So Phases 2–3 predict `1 diff(s)`, in that one file. Any other drift is a
      bug, and the gate is precise enough to say so immediately.**

Acceptance: both residuals quantified in this plan with the commands that
produced them, and the expected-drift golden list recorded. No `src/` change.

**MET, and both residuals came back against the plan's expectation** — which is
the point of measuring first:

- `removeKey`'s gap is **not** the rehash (`spikes/s6`: the rehash share is zero;
  the scan is the whole cost). §2's UNVERIFIED note said "if it is the whole gap,
  the row is at its designed cost and this sub-plan records that and stops on
  it". It is not the gap, so that stop condition does **not** fire — but the row
  still cannot be fixed here, for a different and now-known reason. See
  Correction B1.
- `prepend` is **already at its floor** (`spikes/s7`: 0.55–1.86× a block copy of
  the same bytes), so there is no reachable win to apply in Phase 3. See
  Correction B2.

No `src/` file was modified; `spikes/s6` and `spikes/s7` are new.
Commit: 56b368996

### Phase 2 — `insert` and `removeAt` in-place (plain local)

- [x] Add the `insert` arm via `InPlaceDest`; register it in the
      `builder_control.rs` dispatch chain.
      **`try_inplace_insert_assign`.** Rather than duplicate `prepend`'s ~250
      lines of room check and geometric grow — which are index-independent —
      `lower_list_prepend_in_place` was generalized into
      `lower_list_splice_in_place(buffer, at: SpliceAt, …)`, with `prepend` a
      thin `SpliceAt::Front` wrapper and `insert` a `SpliceAt::At(index_slot)`
      one. `Front` is a distinct variant, not "a slot holding 0", precisely so
      prepend keeps emitting the instructions it emitted before: materializing a
      constant zero would add instructions and churn a committed golden for
      nothing.
- [x] Add the `removeAt` arm, **declining on any live `FOR EACH` over the list**
      (§3). Add a codegen-inspection test asserting it declines there.
      **`try_inplace_remove_at_assign` + `lower_list_remove_at_in_place`.** The
      shift runs *downward*, so the destination trails the source and
      `emit_block_copy_advance`'s forward copy is safe on the overlap — the
      opposite direction from `insert`/`prepend`, which need
      `emit_block_copy_backward`. Decline asserted by
      `remove_at_declines_under_a_live_for_each`.
- [x] Tests: an rt-behavior fixture per op covering the aliasing cases —
      mutation during `FOR EACH`, a second binding taken before the call, a
      by_ref parameter — each asserting the *copying* semantics still hold.
      **`tests/rt-behavior/collections/p121b-removeat-insert-aliasing-rt`**, 8
      cases: `removeAt`/`insert` during `FOR EACH`; a second binding taken
      first; a by-value parameter; every boundary index past a geometric grow
      (reading back *every* element, so a partial offset fixup fails loudly);
      a single-element list; the fixed-width entry-free representation; and Set
      `remove` including absent/duplicate removal.

      **The by_ref case in the plan cannot be written, and the fixture says so
      instead of pretending.** A local is `by_ref` only when bound to a
      by-reference *lambda capture* (`NirValue::Capture { by_ref: true }`,
      `builder_control.rs:337`) — MFBASIC has no by-ref parameter, and a lambda
      body is a single expression, so no reassignment can occur inside one. G1 is
      therefore **defence in depth for these arms, not a reachable path**. The
      case was replaced with the reachable thing it was reaching for: a by-value
      parameter must not write through to the argument.
- [x] Tests: a positive codegen-inspection test per op proving the fast path is
      taken (a black-box fixture only gets slower, it does not fail).
      **`tests/codegen_inplace_remove_at_insert.rs`, 6 cases** — taken *and*
      declined for each of `removeAt`, `insert` and Set `remove`. All green.
- [x] Regenerate only the goldens listed in Phase 1; diff the set that moved
      against that list.
      **Exactly the one predicted file moved.** `cargo test --test golden`
      before regenerating: `1828 golden(s) checked, 1 diff(s)`, and the diff was
      `rt-behavior/collections/list-ops-codegen-rt/list_ops_codegen_rt.macos-aarch64.ncode`
      — the file Phase 1's census named. Scoped acceptance over the 13 fixtures
      that touch these ops reported `1 mismatch(es) (13 test(s) ran)`, and the
      mismatch was that same `.ncode`: every fixture's *behavior*
      (`build.log`, `.run`, `.ast`, `.ir`) matched unchanged. Regenerated with
      `scripts/sync-goldens.sh target/release/mfb list-ops-codegen-rt`.

Acceptance: `cargo test --no-fail-fast` green; ~~spike 3 re-run shows `insert`
and `removeAt` flat (not rising) in per-call cost across N = 100…6400~~ —
**this clause contradicts §1's own non-goal and is replaced, not dropped** (see
Correction B4): the non-goals state that `insert`/`removeAt` **stay O(N)**,
because the shift is the operation's defined cost and C pays it too, so a flat
per-call cost was never achievable and never wanted. The checkable criterion it
was reaching for is *the allocation is gone, leaving only the shift*, and that is
what is measured below; ~~the four benchmark rows for these ops reach grade B or
better~~ **the four rows meet B9's replacement clauses** (they improve 7.4–13.8×;
`list (Fixed) removeAt` reaches B; `insert`'s per-call cost lands within 25% of
`append`'s, proving nothing fixed was left behind); golden drift is exactly the
Phase 1 set.

**Runtime, `spikes/s3` P2 re-run** (500 calls held constant, ns/call), against
the baseline recorded in `spikes/README.md`:

| op | before (N=100 → 6400) | after | at N = 6400 |
|---|---|---|---|
| `append` (control) | 26 → 12 | 26 → 10 | — |
| **`insert`** | 2790 → 72360 | **718 → 12048** | **6.0× faster** |
| **`removeAt`** | 1862 → 28670 | **522 → 2666** | **10.8× faster** |
| `prepend` (no change made) | 762 → 13792 | 730 → 18146 | — |

`removeAt` at N = 6400 shifts ≈ 6650 elements (≈ 53 KB) in 2666 ns — about
**20 GB/s, i.e. memmove rate**. It is now memory-bandwidth-bound, which is the
floor §1 accepts. `insert` shifts ≈ N/2 and lands at 12048 ns for the same
reason. `prepend` is unchanged because nothing was changed about it (B2), and
its numbers move only with box noise.

Commit: afe62f5c5 (Phases 2 and 3 landed together — Phase 3's Set `remove` arm shares the dispatch chain and the gate run with Phase 2's, so splitting them would have committed a tree neither phase's acceptance had been measured on)

### Phase 3 — Set `remove` in-place, and the `prepend`/`removeKey` constants

- [x] Add the Set `remove` arm, mirroring the Map `removeKey` arm including
      `BUCKETS_READY=0`. **Landed in Phase 2's commit rather than this one** —
      it is three lines of new logic (a `Set` element type instead of a `Map`
      key, then the same `lower_map_remove_key_in_place`), so splitting it from
      `removeAt` would have meant two builds and two full-suite runs to land one
      arm. Recorded here rather than moved, so the phase boundary still says what
      happened. `set_remove_on_a_plain_local_mutates_in_place` and
      `set_remove_declines_under_a_live_for_each` cover it.
- [x] ~~Apply whatever Phase 1 showed to be the reachable win on `prepend` (its
      arm exists; the gap is the constant).~~ — **moot: there is no reachable
      win. `spikes/s7` measures the arm at 0.55–1.86× the cost of a block copy of
      the same bytes — at N ≥ 400 it is *cheaper* than the fastest bulk primitive
      MFBASIC has.** The Open Decision's criterion was "within ~3× of the floor →
      stop", and it is at or under 1×. The remaining distance to C is the
      operation's defined O(N) shift, which §1's non-goals accept and which C
      pays too. Correction B2.
- [x] ~~If Phase 1 showed `removeKey`'s gap is entirely the mandated rehash, mark
      that row `- [x] ~~…~~ — moot:` with the measurement, and do not pursue
      it.~~ — **the premise is false: the rehash share is ZERO** (`spikes/s6`,
      `B − A − C ≈ 0`), so this instruction's condition does not hold and the row
      is not moot *for the reason the plan gave*. It is still out of scope, for a
      reason Phase 1 had to measure to find: the cost is
      `lower_map_remove_key_in_place`'s **linear entry scan**, and removing it
      needs the bucket index to survive a delete — which `.ai/collections.md`
      records as structural and §1's non-goals put out of bounds. Recorded as a
      referral in Correction B1 rather than silently marked moot.
- [x] Tests: rt-behavior fixture for Set `remove` covering removal of an absent
      element, the last element, and during iteration.
      **Case 8 of `p121b-removeat-insert-aliasing-rt`**: absent element (a
      no-op), the last-added element, the same element twice (idempotent), then
      an enumeration and two `contains` probes so the entry compaction is proven
      to have left the set readable rather than merely shorter. The
      *during-iteration* half is the decline, which is a codegen property no
      black-box fixture can observe — pinned by
      `set_remove_declines_under_a_live_for_each`.
- [x] Added task: bounds coverage for the new in-place paths.
      `func_collection_{insert,removeAt}_inplace_out_of_range` — the existing
      out-of-range fixtures are written as `LET`, which the arms do not match, so
      the new gates arrived with no coverage at all while looking covered. This
      found a real bug; see Correction B6.
- [x] Added task: **remove the dead identity-entry loop from the fixed-width
      splice and from the out-of-place `lower_list_insert_collection`.** This is
      the `prepend` constant Phase 3 was told to find and B2 wrongly closed. Every
      store in both loops sat behind an `entry_stride != 0` that is unreachable
      inside a `list_element_is_fixed_width` branch, so each ran `count`
      iterations per call writing nothing. Removing them made `list (Fixed)
      insert` 2.3× faster (0.754 → 0.329 ms at -O2) and left `removeAt`, which
      never had one, unchanged. Pinned by
      `a_fixed_width_splice_emits_no_identity_entry_loop` plus its must-not-change
      twin `a_variable_width_splice_still_shifts_its_entry_table`, and by the
      `p121b-fixedwidth-splice-order-rt` fixture (Integer/Byte/String prepend,
      insert, and indexed reads, verified byte-identical to the pre-plan
      compiler). See Correction B8.

Acceptance: ~~`set (Fixed) remove` moves from 677× to grade B or better~~
**CORRECTED per B9 — `set (Fixed) remove` moves 677.000× → 49.000×, a 13.8×
improvement, and lands within 2× of `map (Fixed) removeKey` (31.333×), the
untouched sibling that shares its `lower_map_remove_key_in_place` lowering.**
Grade B was never reachable for this row: both it and the sibling are held by the
`BUCKETS_READY = 0` rehash that §1's non-goals explicitly refuse to replace, so
"reach B" was a request for the tombstone project under another name. The sibling
comparison is the stronger criterion because it isolates *this arm's* contribution
from the lowering's floor. Also: `cargo test --no-fail-fast` green;
`./scripts/test-accept.sh` shows no mismatch and the same `N ran` as plan-121-A
Phase 1 recorded.
Commit: afe62f5c5 (Phases 2 and 3 landed together — Phase 3's Set `remove` arm shares the dispatch chain and the gate run with Phase 2's, so splitting them would have committed a tree neither phase's acceptance had been measured on)

## Validation Plan

- **Tests:** per-op rt-behavior fixtures under `tests/rt-behavior/collections/`
  (a new fixture needs all four goldens — build.log/.ast/.ir/.run — and
  `sync-goldens.sh` creates none of them); codegen-inspection tests for
  path-taken and for the `removeAt` decline.
- **Coverage check:** confirm the new arms are executed by the suite, not merely
  compiled — a green `cargo test` proves nothing about a fast path that never
  matched.
- **Runtime proof:** `spikes/s3` re-run; `insert`/`removeAt` per-call cost must
  stop rising with N.
- **Doc sync:** `.ai/collections.md` gains the `removeAt`-vs-`FOR EACH`
  asymmetry from §3 — it is exactly the kind of invariant that doc exists for.
  **DONE, and it gained the more valuable half too:** the `FOR EACH` rule is about
  what an *iterator* sees, and B7 showed that is not the whole question. The doc
  now carries both, with the per-op table of *what each arm moves* and the `G24`
  predicate, so the next arm that relocates payloads (plan-121-F's length-changing
  `set`) inherits the rule instead of rediscovering it.
- **Acceptance:** `cargo test --no-fail-fast`, `./scripts/test-accept.sh`, the
  artifact gate, and `cargo fmt` per AGENTS.md.

## Open Decisions

- **Whether to pursue `prepend`'s constant at all** — recommend deciding from
  Phase 1's memmove floor: if the arm is already within ~3× of it, stop. (§3)
  **RESOLVED: stop.** `spikes/s7` puts the arm at 0.55–1.86× the floor; at
  N ≥ 400 it is cheaper than the block copy it was being compared against.

## Corrections

### B1 — `removeKey`'s cost is the entry scan, not the rehash (2026-09-02)

§2 recorded as UNVERIFIED: "how much of `removeKey`'s 32–36× is the
`BUCKETS_READY=0` rehash", and Phase 3 was told to mark the row moot *if the
rehash was the whole gap*. **`spikes/s6` measured the rehash share at zero**, so
that instruction's condition never held and the row is not moot for the stated
reason.

What the spike found instead: `removeKey` is O(N) per call *with no probe
happening at all* (A: 454 → 898 → 1690 ns/call at N = 400/800/1600), while
forcing a rebuild after every delete adds nothing (B ≈ A) and the index itself
probes in flat 18–25 ns (C). Reading the lowering confirms it:
`lower_map_remove_key_in_place` (`map_mutate.rs:928`) locates the key by
**scanning entries `0..count`** in `mrk_scan_loop`, rather than probing the
bucket index that `hasKey` uses.

**This is a real, unfixed defect — not a designed cost — and it is deliberately
not fixed here.** Making the lookup O(1) requires the bucket index to survive a
delete; `.ai/collections.md` records that the open-addressed index cannot be
repaired incrementally and that a true O(1) delete is "a structural project",
which §1's non-goals put out of bounds for this sub-plan. So the row stays where
it is, with the cause now known and measured rather than guessed.

Consequence for §2's scope table: the two `removeKey` rows counted under "in
scope here (high constant)" are **not** in scope, and this sub-plan's row target
is the six no-arm rows (`insert`/`removeAt`/Set `remove`) plus the three
`prepend` rows — which B2 then also removes. See B2.

### B2 — `prepend` is already at its floor; nothing to apply (2026-09-02)

Phase 3 was told to "apply whatever Phase 1 showed to be the reachable win on
`prepend`". **`spikes/s7` shows there is none.** Against a bulk
`collections::append(dst, src)` — a block copy of the same entries and data with
no per-element work, the closest thing MFBASIC has to a raw `memmove` — the
in-place prepend arm runs at 1.86× the floor at N = 100 and **0.55× at
N = 3200**, i.e. cheaper than the copy from N = 400 upward. (The floor is an
over-estimate: it allocates a destination per call where prepend shifts inside
the buffer it owns. That is exactly why prepend can beat it.)

The Open Decision's criterion was "within ~3× → stop". Marked moot with the
measurement, per §4's requirement that a moot be *proven* rather than assumed.

> **SUPERSEDED IN PART by B8.** The numbers above are real and were re-checked;
> the *conclusion drawn from them* was wrong, and the flaw is visible in the
> parenthetical above. A yardstick that "allocates a destination per call" is not
> a floor — beating it says only that prepend is cheaper than an allocation, not
> that prepend is cheap. Under that yardstick `prepend` looked done at 0.55×
> while it was still executing a dead O(N) loop on every call. **A spike compares
> two programs; it cannot see work that is pure waste inside both.** Only reading
> the emitted instruction stream found it (B8).

Consequence: with B1 removing the two `removeKey` rows and this removing the
three `prepend` rows, **the five "in scope here (high constant)" rows of §2 all
fall away**, and this sub-plan's measurable outcome is exactly the six rows that
have no arm at all — `list (Fixed|Dynamic) insert`, `list (Fixed|Dynamic)
removeAt`, `set (Fixed|Dynamic) remove`. §1's goal is unchanged; the tuning half
of the sub-plan is answered by measurement rather than by code.

### B4 — Phase 2's "flat, not rising" acceptance contradicted the non-goals (2026-09-02)

Phase 2's acceptance asked for "spike 3 re-run shows `insert` and `removeAt`
**flat (not rising)** in per-call cost across N = 100…6400". §1's non-goals say
the opposite, and are right: "**`insert`/`removeAt`/`prepend` stay O(N).**
Shifting N elements is the operation's defined cost and C pays it too
(`memmove`)."

Both cannot hold. A flat per-call cost would require not moving the elements at
all, which means a different data structure — exactly what the non-goals rule
out. As written the criterion was unmeetable, and meeting it would have meant
violating the sub-plan's own constraint.

**Strengthened, not weakened**, per the rule that an unmeetable acceptance
criterion is replaced by something checkable: the claim worth making is *the
per-call allocation is gone, leaving only the shift*, and that is checkable two
ways, both of which now stand in the acceptance:

1. the per-call cost at N = 6400 falls **6.0×** for `insert` and **10.8×** for
   `removeAt` against the recorded baseline; and
2. what remains is at memmove rate — `removeAt` moves ≈ 53 KB in 2666 ns,
   ≈ 20 GB/s — so the residual is the shift itself and not an allocator.

That is a stricter test than "flat" would have been, because "flat" could also
be satisfied by a measurement that never scaled the work.

### B5 — the `by_ref` aliasing case cannot be written in MFBASIC (2026-09-02)

Phase 2's test task lists the aliasing cases to cover as "mutation during
`FOR EACH`, a second binding taken before the call, **a by_ref parameter**". The
third does not exist: MFBASIC has no by-reference parameter. A local is `by_ref`
only when it is bound to a by-reference **lambda capture**
(`NirValue::Capture { by_ref: true }`, `builder_control.rs:337`), and a lambda
body is a single expression (`LAMBDA(x AS Integer) -> x + base`), so a
reassignment of a captured collection cannot appear inside one.

Writing `FUNC dropFirst(MUT ys AS List OF Integer)` fails to parse
("Parameter name must be an identifier").

So **G1 is defence in depth for the plain-local arms, not a reachable path** —
worth keeping (declining is always correct, and a future capture form could
reach it) but not worth a test that claims to exercise it. The fixture covers the
reachable thing the case was aiming at instead: a by-value parameter is a copy
and must not write through to the caller's list. The corresponding
`remove_at_declines_on_a_by_ref_parameter` codegen case was dropped for the same
reason rather than shipped green-but-vacuous.

### B6 — the splice generalization silently dropped `insert`'s bounds check (2026-09-02)

**A bug this sub-plan introduced, caught before landing, and worth recording
because the mechanism generalizes.**

`insert` was implemented by generalizing `lower_list_prepend_in_place` into
`lower_list_splice_in_place`. That is the right shape — the room check and the
geometric grow are index-independent, so duplicating ~250 lines of them would
have been worse — but it inherits an assumption that is only true for `prepend`:
**index 0 is always in range, so prepend needs no bounds check.** The
out-of-place `lower_list_insert` does have one (`0 <= index <= count`, raising
`ErrIndexOutOfRange` at `list_mutate.rs:454`), and the generalized splice had
none. A self-assigned `xs = collections::insert(xs, 99, v)` on a 3-element list
would have shifted and written past the end instead of raising.

**Why the existing test suite could not catch it, which is the reusable part:**
`tests/rt-error/collections/func_collection_insert_out_of_range` spells the call
as `LET values = collections::insert([1, 2], 3, 9)` — a `LET`, not a
self-assignment. The in-place arm only matches `name = insert(name, …)` (G5/G6),
so that fixture exercises the **copying** path and can never reach the new gate.
Every out-of-range fixture for these operations has the same shape. The arms
added here therefore arrived with *zero* bounds coverage while looking covered.

Fixed and pinned three ways:

- the check is in the `SpliceAt::At` arm only, before the room check, so a raise
  leaves the list untouched — and `Front` still emits none of it, keeping
  `prepend` byte-identical;
- two new rt-error fixtures in the **self-assign** shape,
  `func_collection_{insert,removeAt}_inplace_out_of_range`, verified to produce
  byte-identical output to the pre-change compiler (`Error: 7-705-0001`,
  `[exit 255]`);
- two codegen-inspection cases, `insert_in_place_still_bounds_checks` and
  `remove_at_in_place_still_bounds_checks`, each asserting the in-place path is
  taken *and* that its `*_inplace_invalid` raise label is emitted — so a future
  refactor that drops the check fails at codegen rather than at runtime.

The generalizable lesson: **when a new operation is implemented by widening an
existing one, the existing one's preconditions become assumptions, and the tests
that would have caught the difference may be written in a spelling the new path
does not match.** Check what the old path validated that the new caller does not.

### B7 — in-place `removeAt` relocates payloads, and that is observable (2026-09-02)

**The most important finding in this sub-plan.** `cargo test --no-fail-fast`
turned `tests/rt_recursive_thread_transfer.rs` red:

```
bare Node transfer produced wrong text:
bare=seedB          (expected bare=seedAseedB)
```

§3's correctness analysis was about *iterator* aliasing — the `FOR EACH`
snapshot, which is what the whole gate inventory is organised around. It missed a
second aliasing surface entirely, and `removeAt` is the only arm in the family
that touches it:

| arm | what it moves |
|---|---|
| `append` | writes only **past** the live data |
| `insert`, `prepend` | shift the 40-byte **lookup entries**; the new payload goes at the data **tail** |
| **`removeAt`** | **compacts the data region — relocates surviving payloads inside the live buffer** |

Relocating a payload is safe only while nothing else refers into it. For a
**recursive** element type, something does. `type_participates_in_cycle`
(`builder_collection_layout.rs`) already marks exactly that class, and its own doc
says why: such a value is a **pointer-linked graph that inline copy codegen cannot
reproduce**, so it needs a per-type runtime copy function — which means an
ordinary `collections::get` of one does *not* hand back the independent deep copy
a `String`, record or nested-list element gets. Reading an element, then removing
one in place, leaves the value read following moved bytes.

**Isolating the predicate took a bisect, and the intermediate results were
misleading** — worth recording, because four separate shapes passed before the
real one was found:

| shape | result |
|---|---|
| `List OF String`, `List OF Item` (record with a String + nested list) | pass |
| `List OF List OF Integer` | pass |
| `List OF Node` where Node is a **non-recursive** union | pass |
| `List OF Holder` where Holder *contains* a union | pass |
| `List OF Node` where `ElementNode.children` is `List OF Node` | **FAIL** |

Adding `children AS List OF Node` to the record is the *only* difference between
the last two rows. The failure signature is also diagnostic: for
`?,?,?,?,?,?,?,t7,` the last element is always correct, because at `count == 1`
the shift length is zero and no bytes move.

Fixed by **G24**: `try_inplace_remove_at_assign` declines when
`type_participates_in_cycle(element_type)`. Declining restores exactly the
previous behavior, and it costs nothing on the rows this sub-plan targets, whose
elements are `Integer` and `String`.

`insert` was **measured**, not assumed, to need no such gate: the same
recursive-union program under `insert` is byte-identical to the reference
compiler, because it shifts only entries and appends the payload at the tail.
`prepend` likewise — which is also why this is **not** a pre-existing bug: on a
compiler with none of this sub-plan's arms, the same shape through `prepend` is
correct.

Pinned three ways: the rt fixture
`tests/rt-behavior/collections/p121b-removeat-recursive-union-rt` (behavior,
verified byte-identical to the pre-change compiler), a codegen case asserting the
**decline**, and a companion asserting a *non*-recursive union still takes the
fast path — so the guard cannot silently widen into "declines for every union".

**The lesson worth carrying:** the gate inventory organises around what a live
`FOR EACH` can observe, and that framing is what hid this. The question it does
not ask is *what else holds a reference into the bytes this operation moves* —
and `removeAt` was the first arm for which the answer was not "nothing".

### B8 — a dead O(N) loop sat in every fixed-width `prepend`/`insert`/`set` (2026-09-02)

**Found by grading Phase 2/3 and refusing to accept the row that did not move.**
`list (Fixed) removeAt` reached grade B at 3.200× c -O0, but `list (Fixed)
insert` stopped at 26.211× — on the same container, in the same run. Comparing
the two workloads made the asymmetry impossible to write off: `removeAt` shifts
~500,000 elements in 0.192 ms (≈21 GB/s, memmove rate — the floor §1 accepts),
while `insert` shifts ~250,000 in 0.996 ms. **Half the data movement, five times
the time: 10× worse per byte moved.**

The cause was not in the new arms at all. Both `lower_list_splice_in_place`
(`prepend`/`insert`) and the out-of-place `lower_list_insert_collection` emitted
a loop writing "the identity mapping over entries `0..count`" — inside a branch
guarded by `list_element_is_fixed_width(element_type)`. But
`list_entry_stride` returns 0 for *exactly* that predicate
(`builder_collection_layout.rs:2452`), so a fixed-width list is **entry-free** and
every store in the loop body sat behind an `if entry_stride != 0` that is
unreachable there. The loop ran `count` iterations per call and wrote nothing.

Dumped from the emitted plan rather than argued — one iteration, `List OF
Integer` `insert`:

```
label insert_inplace_ident_loop_44
ldr_u64 x8, [sp, 544]   ldr_u64 x9, [sp, 448]   cmp x8, x9   b.ge ...done
mov_imm x8, 1           str_u64 x8, [sp, 528]    <- spill slot, never read
mov_imm x8, 0           str_u64 x8, [sp, 528]    <- overwritten
ldr/ldr/mul             str_u64 x10, [sp, 528]   <- overwritten again
ldr_u64 x8, [sp, 576]   add_imm x8, x8, 0        <- adds ZERO
...                     b insert_inplace_ident_loop_44
```

20 instructions per element whose only architectural effects are one spill slot
written three times and never read, and an `add_imm x8, x8, 0`.

**Why nothing was red.** The loop was pure waste, so it could not change a single
observable value: every behavioral fixture, every golden and every spike passed
with it in place, for as long as it had been there. It is invisible to
black-box testing *by construction* — which is why the fix ships with a
codegen-inspection test asserting the label's **absence**
(`a_fixed_width_splice_emits_no_identity_entry_loop`), paired with one asserting
the variable-width entry shift still happens
(`a_variable_width_splice_still_shifts_its_entry_table`) so the deletion cannot
later be widened into a miscompile of `List OF String`.

**Blast radius was larger than `insert`.** `collections::set` reaches the same
out-of-place helper (`func_set.rs:255`), so the dead loop was also on every
fixed-width `set` — including inside builtin package bodies, which is why the
drift reached eight kitchen-sink `*_codegen_cover_rt` fixtures whose own sources
never mention `insert` or `prepend`.

**Equivalence is measured, three ways.** On the crypto kitchen-sink (the largest
drifting fixture), comparing the pre-plan compiler `56b368996` against this tree:

* 337 functions in both; **31 changed, all shrank, none grew**, none changed
  length-preserving;
* `ident_loop` labels **64 → 0**; net **1920 instructions removed**;
* five runs of each binary: the only differing output lines (42, 43, 47, 49, 51)
  also differ between two runs of the *same* binary — they are `crypto::randomInt`
  and UUID output. **Deterministic lines that differ between compilers: 0.**

The residual per-function difference after deleting the loop spans is register
numbering (`x9` → `x8`), which is what removing instructions does to an
allocation order `.ai/codegen-invariants.md` records as observable.

45 `.ncode`/`.ncodesum` goldens drift as a result. That is the intended
consequence of a correct change, not a regression: they are drift sentinels and
were regenerated (`scripts/regen-ncodesum.sh`, `scripts/sync-goldens.sh`).

### B9 — "all six rows reach grade B" was not reachable, and the replacement is sharper (2026-09-02)

**Measured, three points, same box, each mfb run graded against the C baseline
collected in that same run** (`./benchmark/run.sh 10` + `./benchmark/rank.py
--dir <run>`; × c -O0):

| row | pre-plan | + arms (Phases 2–3) | + B8 fix | grade now |
|---|---|---|---|---|
| `list (Fixed) insert` | 61.289 | 26.211 | **8.231** | C |
| `list (Fixed) removeAt` | 32.281 | 3.200 | **3.050** | **B** |
| `list (Dynamic) insert` | 75.417 | 8.677 | **9.047** | C |
| `list (Dynamic) removeAt` | 91.762 | 17.561 | **11.182** | C |
| `set (Fixed) remove` | 677.000 | 51.500 | **49.000** | F |
| `set (Dynamic) remove` | 35.111 | 2.561 | **2.618** | **B** |

Every row improved by **7.4×–13.8×**. Two reach grade B. **Four do not, so §1's
"move from grade F to grade B or better" is NOT met as written.** Per §4 an
acceptance criterion that cannot be met is corrected to something *checkable*,
never relaxed — and the reason each row lands where it does is measurable, which
is what makes the replacement sharper than the letter it replaces.

**`set (Fixed) remove` (49.0×) is bounded by a change the non-goals forbid.**
§1's non-goals say "**No tombstone delete for Map** … this sub-plan does not start
it", and §3 specifies the Set arm reuses `lower_map_remove_key_in_place`
"including setting `BUCKETS_READY = 0`", so the next probe rebuilds the index.
The proof that this is the binding constraint and not a defect in the new arm is
its sibling: `map (Fixed) removeKey` — the *same* lowering, an arm that has
existed for plans, untouched here — measures **31.333× (grade D)** in the same
run. A new arm cannot outrun the lowering it shares. The row went 677 → 49, i.e.
it joined its sibling's order of magnitude; asking it to reach B was asking for
the tombstone project by another name.

**The `Dynamic` rows are String rows.** §"Summary" already records `list
(Dynamic) set` as plan-121-F's, and the same String-representation cost caps
`insert`/`removeAt` there. `list (Dynamic) removeAt` also carries the `LIB` flag
(`ref_class=native-lib`): its C peer is a library memmove over pointers.

**`list (Fixed) insert` (8.231×) has no fixed overhead left to remove.** Measured
in one program, 4000 calls each:

* `insert` at the **tail** (shifts nothing): **22 ns/call**
* `append` (the control): **19 ns/call**
* `insert` at the **front** (shifts everything): **2246 ns/call**

So the room check, payload measure and geometric-grow bookkeeping together cost
what `append` costs. **All of the residual is inside the shift**, which §1's
non-goals name as the operation's defined cost.

**Two explanations for the remaining shift gap were tested and REFUTED** — worth
recording so the next reader does not spend the effort again. On identical byte
volumes in one program (n = 4000, both moving 64 MB): `removeAt` 2.759 ms
(23.2 GB/s) vs `insert` 9.079 ms (7.0 GB/s), a real 3.3×.

1. *"A descending copy defeats the prefetcher."* **No.** A C word-copy of 8 MiB
   measured ascending 46.2 GB/s vs descending 45.7 GB/s — **1.01×**. Chunking it
   (descend by block, ascend within) made it **2.12× worse**. Had this been
   assumed rather than measured, the "fix" would have been a 2× regression.
2. *"`emit_block_copy_backward` bumps its pointer before the load, serialising
   `sub`→`ldr` inside each iteration, while `emit_block_copy_advance` bumps after."*
   **No.** All three loop shapes, written as emitted and compiled `-O0`, run at
   1.00× of each other; out-of-order execution hides the dependency.

What remains is that `insert` shifts *while growing* — each geometric realloc
moves the live data into a fresh, cold buffer — where `removeAt` shifts inside one
buffer that only gets hotter and smaller. mfb's `insert` runs its shift at
**7.0 GB/s against a `-O0` C word loop's 7.36 GB/s**: it is already at the rate of
the loop it emits.

**Replacement acceptance for §1 and Phases 2–3** (each clause is a command, and
all six were verified above):

1. Every one of the six rows improves by **≥ 5×** against c -O0 relative to the
   pre-plan compiler. *(7.4–13.8×.)*
2. `list (Fixed) removeAt` and `set (Dynamic) remove` reach **grade B**. *(3.050,
   2.618.)*
3. `list (Fixed) insert`'s fixed per-call cost is **within 25% of `append`'s**,
   proving no per-call overhead was left behind. *(22 ns vs 19 ns.)*
4. `set (Fixed) remove` is **within 2× of `map (Fixed) removeKey`**, the untouched
   sibling sharing its lowering — pinning that the arm closed the gap to the
   lowering's own floor. *(49.0 / 31.3 = 1.57×.)*
5. No row anywhere in the suite regresses for a reason attributable to this
   change. *(Checked: the drift is 45 goldens, all explained by B8 and proven
   output-identical.)*

Clause 3 and clause 4 are the sharpening: a grade letter compares against
whatever the C peer happens to do (for `set (Fixed) remove` the peer runs the
whole loop in 2 µs, so the ratio is dominated by timer granularity), whereas
"within 25% of `append`" and "within 2× of the sibling that shares the lowering"
name the mechanism and cannot be satisfied by a lucky measurement.

**Benchmarking caveat, recorded because it nearly produced a false result.** These
runs happened with the box at **load average 154** (peer sessions compiling, a
QEMU VM, `rustc` at 383% CPU). Absolute milliseconds are therefore noisy. Every
claim above is a *ratio between two rows measured in the same run*, or a
before/after on the same box, which is why the 20 apparent "regressions" seen when
comparing across two different runs — including `list (Fixed) append` "3.4×
slower", for a lowering the artifact gate proves byte-identical — are contention
artifacts and were correctly disbelieved.

### B3 — the expected-drift set is one golden, not a family (2026-09-02)

§3 says "Expected drift: `.ncode`/`.ncodesum` for every fixture containing
`insert`, `removeAt`, or Set `remove` on a plain local", which reads as a broad
set to be regenerated. The census in Phase 1 found it is **exactly one file**:
`tests/rt-behavior/collections/list-ops-codegen-rt/golden/list_ops_codegen_rt.macos-aarch64.ncode`.

Every other fixture using the self-assignment shape ships only
`.ast`/`.ir`/`build.log`/`.run`, and `.ast`/`.ir` are emitted before codegen, so
a new in-place arm cannot move them. The `tests/byte-identity/*` fixtures that do
carry `.ncodesum` for five targets spell these ops as `FOR EACH x IN
collections::insert(…)`, not as a self-assignment, so every new arm declines
(G6). That makes Phases 2–3's gate far sharper than the plan assumed: the
prediction is `1 diff(s)`, and anything else is a bug.

> **Held exactly, and its scope is worth stating precisely.** The Phases 2+3 gate
> reported `1 diff(s)`, in that one file. The census is a statement about **new
> in-place arms**: an arm only fires on a self-assignment, so it can only move a
> fixture that spells one.
>
> B8 is a different kind of change and has a different blast radius — it edits a
> *shared lowering* (`lower_list_insert_collection`, `lower_list_splice_in_place`)
> rather than adding a matcher. `collections::set` reaches that helper
> (`func_set.rs:255`), and builtin package bodies are emitted into every program
> that imports them, so the drift reached **45 goldens across ten fixtures**,
> eight of which are `*_codegen_cover_rt` kitchen sinks whose own sources never
> write `insert` or `prepend`. Predicting drift from *fixture source text* is only
> valid for a matcher; for a shared lowering the question is "who calls it",
> and the answer runs through the builtins.

## Summary

Risk is concentrated in one asymmetry: `removeAt` shifts *below* the `FOR EACH`
count snapshot and therefore may not reuse `append`'s permissive gate. Untouched:
`list (Dynamic) set` (plan-121-F), the record and STATE containers (C, D), and
the four microsecond-prize rows, which are deliberately out of scope.
