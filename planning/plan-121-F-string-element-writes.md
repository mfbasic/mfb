# plan-121-F: Length-changing String element writes

Last updated: 2026-09-02
Effort: large (3h–1d)
Depends on: plan-121-D

`collections::set` on a `List OF String` is O(1) when the replacement has the
same byte length as the element it replaces, and catastrophically worse than
linear when it does not — measured at **O(N^1.6)**, not the O(N) a data-region
shift would cost. `list (Dynamic) set` runs 371× c -O0 for this reason, and the
same cost caps the four other `Dynamic`-container `set` rows that plan-121-C and D
otherwise fix.

Behavioral outcome: a length-changing element write costs one bounded shift
inside the existing block, with no per-call allocation, so its cost is linear in
the bytes after the element and nothing else.

References: `.ai/collections.md` §"List memory management" (headroom, the
`capacity`-not-`count` data-base rule, `copy_collection_tight`);
`benchmark/README.md` §"Arena-churn caveat"; `mfb spec` §14.

## Prerequisites

Stated once in plan-121-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-121-D complete and archived | `ls planning/completed/plan-121-D-*` → 1 file | NOT MET |

If plan-121-D is not complete, this sub-plan cannot start, full stop. (The
container work must land first: otherwise a `Record-Dynamic` row improved here is
still dominated by the whole-collection copy and the win cannot be measured.)

## 1. Goal

- A length-changing `set` on a `List OF String` performs a bounded shift within
  the existing block, allocating only when the data region genuinely overflows
  its `dataCapacity`.
- The five rows in §2 reach grade B or better; spike 2's "longer" and
  "shorter" variants become linear in N rather than O(N^1.6).

### Non-goals

Inherited verbatim from plan-121-A §1. Additionally:

- **No change to the collection block layout or the canonical String form.**
  Entry stride, header fields, the `[len][bytes][NUL]` string shape (D9) and the
  `capacity`-based data-region base all stay exactly as they are. This sub-plan
  changes *when bytes move*, not *how they are laid out*.
- **No indirection/pointer representation for String elements.** Storing
  `char*`-style pointers per element (which is what C does, and why C is O(1)
  here) would be a layout and ABI change, would break `copy_collection_tight`'s
  tight-copy contract, and is explicitly out of scope. **The O(N) shift is
  accepted as the floor** (§3).
- **No change to `copy_collection_tight`.** Headroom must still never leak into
  a snapshot or across a thread boundary.

## 2. Current State

List elements of variable-length type are stored as packed bytes in one data
region sized by `dataCapacity`, with the region based at
`header + capacity*ENTRY` (`.ai/collections.md`: "data base uses capacity, never
count"). `try_inplace_set_assign` (`builder_inplace_assign.rs:469`) matches
`x = collections::set(x, i, v)` on a plain local and mutates in place — but only
the fixed-width entry write is genuinely in place. When `v`'s byte length differs
from the element being replaced, every later element's bytes must move.

### Measured populations

| What | Count | Command |
|---|---|---|
| Rows blocked on this (String `set`, all containers) | 5 | `./benchmark/rank.py --csv \| awk -F, '$2=="set" && $1 ~ /Dynamic/'` |
| …namely | `list (Dynamic)` 371×, `map (Record-Dynamic)` 146×, `map (State-Dynamic)` 580×, `list (Record-Dynamic)` 677×, `list (State-Dynamic)` 17742× | same |
| Worst | 17742× (`list (State-Dynamic) set`) — shared with plan-121-D | same |
| Plain-local String `set` (this sub-plan's own row) | 371× | `list (Dynamic) set`, firm confidence |

### Verified properties

- **VERIFIED — same-length writes are already O(1); any length change is not.**
  Spike 2 holds the call count at 2000 on a plain `MUT List OF String` and varies
  N ∈ {50, 200, 800, 3200}:
  - replacing 5 bytes with 5: **40, 40, 35, 43 ns/set** — flat.
  - replacing 5 bytes with 6: **173, 1647, 12661, 155037 ns/set**.
  - replacing 5 bytes with 4: **73, 891, 13056, 217819 ns/set**.
  Shorter is as bad as longer, so this is not "needs more room" — it is "the
  length differs at all".
- **VERIFIED — the growth is worse than the shift explains.** From N=50 to
  N=3200 (64×), the longer-write cost rises 896×, i.e. ≈ O(N^1.6). A pure
  data-region `memmove` would be O(N). The excess is consistent with the
  documented arena behaviour: `benchmark/README.md` records that the runtime
  arena's free list "degrades quadratically under mixed-size transient churn",
  and a per-call reallocation of a growing data region is exactly that churn.
- **VERIFIED — the achievable target is worth the work.** At the benchmark's
  size (200 elements × ~5 bytes, 2000 sets) a pure shift moves ≈ 2 MB total,
  which is ≈ 0.1 ms of `memmove`. C takes 0.041 ms. So a correct shift-in-place
  implementation lands near **2–3× C (grade A/B)**, from 371× today — without
  any layout change.

## 3. Design Overview

Two independent costs, addressed in order:

1. **The allocation.** Today a length-changing write reallocates the data region
   per call. The block already carries `dataCapacity > dataLength` headroom
   (`.ai/collections.md`). A write whose new length fits within the existing
   `dataCapacity` must shift the tail inside the current block and update
   `dataLength` — no allocation, no free, no arena churn. Only a genuine
   `dataCapacity` overflow takes the existing geometric grow path.
2. **The shift.** With the allocation gone the cost is one `memmove` of the bytes
   after the element, which is the accepted floor (§1 non-goals).

**Where correctness risk concentrates:** the data region's offsets. Every entry
after the modified element has its byte offset changed, so the entry table must
be fixed up in the same operation. Getting this half-right produces a collection
that *reads correctly for the first i elements* and returns garbage after — a
failure mode that a checksum-only test can miss if the checksum happens to fold
only early elements. **Tests must read back every element**, not a fold.

The second risk is `copy_collection_tight`: shifting inside a block that carries
headroom must leave the block in a state the tight copy still normalizes
correctly, or headroom leaks into a snapshot.

**Byte-identity is NOT the gate.** Expected drift: `.ncode`/`.ncodesum` for every
fixture containing a String-element `set`. Phase 1 records the set. Note also
that `tests/rt-behavior/collections/collection-set-string-grow-rt` exists and is
directly about this path — read it before changing anything.

### Rejected alternatives

- **Per-element pointers (C's representation).** Rejected in the non-goals: it
  makes `set` O(1) but is a layout/ABI change and breaks the tight-copy contract.
- **Per-element slack in the data region.** Rejected: it makes the common
  same-length case no faster (already O(1)), costs memory on every list, and
  still needs the shift when slack is exhausted.
- **Leave it and document it.** Rejected: 4 rows, one of them the worst in the
  suite, and the fix needs no layout change.

## Phases

### Phase 1 — Characterize the current path and pin the failure mode

- [ ] Read `tests/rt-behavior/collections/collection-set-string-grow-rt` and
      record what it already protects; this path has a regression test and the
      four-question gate applies if it needs to change.
- [ ] Write an rt fixture that sets every element of a `List OF String` to a
      different-length value and reads **all** of them back (not a fold), so a
      partial offset fixup fails loudly. Confirm it passes at HEAD.
- [ ] Record the `.ncode` goldens containing String-element `set`.

Acceptance: the read-back-everything fixture exists and passes at HEAD; the
expected-drift golden list is recorded. No `src/` change.
Commit: —

### Phase 2 — Shift within `dataCapacity` instead of reallocating

- [ ] In the `set` lowering, when the replacement length differs and the new
      `dataLength` fits `dataCapacity`: shift the tail bytes, fix up every later
      entry's offset, update `dataLength`. No allocation.
- [ ] Keep the existing geometric grow path for genuine overflow.
- [ ] Tests: the Phase 1 fixture, plus cases for shorter/longer/same, element 0,
      the last element, and a single-element list.
- [ ] Tests: a case that overflows `dataCapacity` mid-loop, proving the grow path
      still works and the tight copy still normalizes.

Acceptance: spike 2's "longer" and "shorter" variants become linear in N (per-set
cost rising ~linearly rather than the measured 896× over a 64× size range);
`cargo test --no-fail-fast` green; golden drift confined to the Phase 1 set.
Commit: —

### Phase 3 — Confirm the container rows unblock

- [ ] Re-run `./benchmark/run.sh 10` and re-rank.
- [ ] Confirm `list (Dynamic) set` reaches grade B or better and that the
      `Record-Dynamic` / `State-Dynamic` `set` rows fixed by plan-121-C/D are no
      longer capped by this cost.
- [ ] If `list (State-Dynamic) set` is still an outlier, record the residual and
      whether it belongs to this sub-plan or D.

Acceptance: the five String-`set` rows in §2 each reach grade B or better, or the
residual is documented with a measurement naming which sub-plan owns it.
Commit: —

## Validation Plan

- **Tests:** rt-behavior fixtures reading back every element (a new fixture needs
  all four goldens — build.log/.ast/.ir/.run — and `sync-goldens.sh` creates
  none); the existing `collection-set-string-grow-rt` must pass unchanged.
- **Coverage check:** confirm the new shift path is executed, and specifically
  that both the fits-in-`dataCapacity` and the overflow branches are covered — a
  test that only ever takes one branch leaves the other unproven.
- **Runtime proof:** spike 2 re-run; "longer" and "shorter" must track "same"
  in shape (linear, not super-linear).
- **Doc sync:** `.ai/collections.md` gains the length-changing-write rule and the
  entry-offset fixup obligation.
- **Acceptance:** `cargo test --no-fail-fast`, `./scripts/test-accept.sh`, the
  artifact gate, `cargo fmt` per AGENTS.md.

## Open Decisions

- **Whether to shift-and-fix-up, or rebuild the data region in place** — a full
  in-block rebuild is simpler to get right (rewrite all entries and bytes from
  scratch into the same block) and still allocation-free, at the cost of touching
  all N bytes rather than only the tail. Recommend starting with the simpler
  rebuild, measuring, and only doing the tail-only shift if the benchmark still
  falls short of grade B. Correctness first. (§3)

## Corrections

<Filled in during execution.>

## Summary

The risk is entirely in entry-offset fixup, whose failure mode is silent garbage
past the modified element — which is why the tests read back every element rather
than folding a checksum. The O(N) shift itself is accepted as the floor; the win
is deleting the per-call allocation and the arena churn it causes, which
arithmetic in §2 puts at 371× → ~2–3× C without touching layout.
