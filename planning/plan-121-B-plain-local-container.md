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
- The six rows in §2 move from grade F to grade B or better, measured by
  `./benchmark/rank.py`.

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
Commit: —

### Phase 2 — `insert` and `removeAt` in-place (plain local)

- [ ] Add the `insert` arm via `InPlaceDest`; register it in the
      `builder_control.rs` dispatch chain.
- [ ] Add the `removeAt` arm, **declining on any live `FOR EACH` over the list**
      (§3). Add a codegen-inspection test asserting it declines there.
- [ ] Tests: an rt-behavior fixture per op covering the aliasing cases —
      mutation during `FOR EACH`, a second binding taken before the call, a
      by_ref parameter — each asserting the *copying* semantics still hold.
- [ ] Tests: a positive codegen-inspection test per op proving the fast path is
      taken (a black-box fixture only gets slower, it does not fail).
- [ ] Regenerate only the goldens listed in Phase 1; diff the set that moved
      against that list.

Acceptance: `cargo test --no-fail-fast` green; spike 3 re-run shows `insert` and
`removeAt` flat (not rising) in per-call cost across N = 100…6400; the four
benchmark rows for these ops reach grade B or better; golden drift is exactly the
Phase 1 set.
Commit: —

### Phase 3 — Set `remove` in-place, and the `prepend`/`removeKey` constants

- [ ] Add the Set `remove` arm, mirroring the Map `removeKey` arm including
      `BUCKETS_READY=0`.
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
- [ ] Tests: rt-behavior fixture for Set `remove` covering removal of an absent
      element, the last element, and during iteration.

Acceptance: `set (Fixed) remove` moves from 677× to grade B or better;
`cargo test --no-fail-fast` green; `./scripts/test-accept.sh` shows no mismatch
and the same `N ran` as plan-121-A Phase 1 recorded.
Commit: —

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

Consequence: with B1 removing the two `removeKey` rows and this removing the
three `prepend` rows, **the five "in scope here (high constant)" rows of §2 all
fall away**, and this sub-plan's measurable outcome is exactly the six rows that
have no arm at all — `list (Fixed|Dynamic) insert`, `list (Fixed|Dynamic)
removeAt`, `set (Fixed|Dynamic) remove`. §1's goal is unchanged; the tuning half
of the sub-plan is answered by measurement rather than by code.

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

## Summary

Risk is concentrated in one asymmetry: `removeAt` shifts *below* the `FOR EACH`
count snapshot and therefore may not reuse `append`'s permissive gate. Untouched:
`list (Dynamic) set` (plan-121-F), the record and STATE containers (C, D), and
the four microsecond-prize rows, which are deliberately out of scope.
