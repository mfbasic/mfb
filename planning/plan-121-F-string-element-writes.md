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
| plan-121-D complete and archived | `ls planning/completed/plan-121-D-*` → 1 file | **MET** — 1 file (`plan-121-D-state-field-container.md`), command re-run rather than trusting the column |

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

- [x] Read `tests/rt-behavior/collections/collection-set-string-grow-rt` and
      record what it already protects.

      **It protects five things** (plan-02 Phase 1): a fixed-width `set` at scale
      (1000 elements, no realloc); a same-size **record** `set` keeping its String
      field intact across 10 passes; a variable-width `set`, shorter then longer;
      `FOR EACH` safety, i.e. an in-place `set` staying invisible to a live
      iterator; and String self-append with copy-freeze independence.

      **It does not need to change**, so the four-question gate is not engaged:
      Phase 2 changes *when bytes move*, not what the list reads back, and this
      fixture asserts only observable values. It must keep passing **unchanged**,
      which is the point of leaving it alone.

      **But it is not sufficient**, which is why Phase 1 asks for another: its
      variable-width case is a **four-element** list (`["alpha","beta","gamma",
      "delta"]`, two writes). A partial entry-offset fixup — the failure mode §3
      names — can easily survive four elements and two writes.
- [x] Write an rt fixture that sets every element of a `List OF String` to a
      different-length value and reads **all** of them back (not a fold), so a
      partial offset fixup fails loudly. Confirm it passes at HEAD.
      **`tests/rt-behavior/collections/p121f-string-set-readback-rt`.**

      It reports the **first mismatching index**, not a count and not a fold, so
      a partial fixup names where it broke. Element `i` is given a value that is
      distinct per index *and* of index-dependent length (`i MOD 7 + 1` repeats),
      so neighbouring elements differ in length and a shift landing one byte off
      cannot coincidentally still match. Seven cases:

      | case | what it catches |
      |---|---|
      | 200 writes, every one LONGER | the ordinary grow-and-shift |
      | 200 writes, every one SHORTER | spike 2 found shorter as bad as longer |
      | 200 writes **backwards** | the tail is already long when the head moves |
      | element 0 and the last element | the two ends of the shift |
      | a single-element list | the degenerate case with no tail to shift |
      | 6 passes over 60 elements, each longer than the last | forces the `dataCapacity` overflow path **repeatedly**, not once |
      | a copy taken before the writes | the `copy_collection_tight` contract — the snapshot must not observe them |

      **Confirmed passing at HEAD**: `firstBad=-1` on every pass. That is the
      point of landing it before Phase 2 — today's path is *correct and slow*, so
      this fixture pins the correctness that the optimization must not trade away.
- [x] Record the `.ncode` goldens containing String-element `set`.

      **The set is `tests/byte-identity/collections` — one root, and unlike
      plan-121-D's it is NOT empty.** Census (`/tmp/p121f-census.sh`, walking each
      `.ncodesum` up to its owning `project.json`):

      | query | result |
      |---|---|
      | roots that resolve (control) | **141 of 141** |
      | roots mentioning `collections::set` (control) | 2 — `byte-identity/collections`, `byte-identity/http` |
      | roots with **both** `List OF String` and `collections::set` | **1 — `byte-identity/collections`** |

      Widening past `.ncode`, ten fixtures tree-wide combine the two; their
      `.run`/`build.log` goldens must **not** move, since Phase 2 changes cost and
      not observable values. Any movement there is a bug, not drift.

Acceptance: **MET.** `p121f-string-set-readback-rt` exists and passes at HEAD
(`firstBad=-1` on all three 200-element passes and the overflow pass). The
expected-drift list is recorded above and is a single root. No `src/` change in
this phase.
Commit: —

### Phase 2 — Shift within `dataCapacity` instead of reallocating

- [x] In the `set` lowering, when the replacement length differs and the new
      `dataLength` fits `dataCapacity`: shift the tail bytes, fix up every later
      entry's offset, update `dataLength`. No allocation. **Done** — the
      `set_inplace_shift` path, with the two directions kept separate because they
      are not the same code: widening moves the tail **up** into itself and needs a
      **backward** copy (a forward one smears the first tail bytes over the region
      whenever the shift distance is less than the tail length — and still looks
      correct on a 1–2 element list), narrowing moves it **down** and needs a
      forward one.

      The offset fixup needed a new helper. `emit_offset_compaction_fixup`
      subtracts, and the two directions are **not** the same operation with a
      negated argument — offsets are read back unsigned, so a negative `hole_len`
      would wrap. `emit_offset_expansion_fixup` is its mirror. Both use `>` rather
      than `>=`, which is what leaves the written element's own entry alone: its
      `valueOffset` is exactly the span start and does not move.
- [x] ~~Keep the existing geometric grow path for genuine overflow.~~ — **there
      was no such path; it had to be written.** See Correction F1: the overflow
      fell back to the `removeAt` + `insert` rebuild, which produces a **tight**
      buffer, so the next widening overflowed again and the shift never ran. New
      helper `emit_grow_list_data_capacity` grows the data region geometrically
      (`emit_geometric_step`, as `append` does) and carries on shifting.

      Simpler than `append`'s grow by construction: `capacity` is unchanged, so
      the header, the entry table and the live data are one **contiguous** prefix
      and it is a single verbatim block copy — and the data region keeps the same
      block-relative base (`.ai/collections.md`: "data base uses capacity, never
      count"), so **no entry offset moves** and there is nothing to fix up.
- [x] Tests: the Phase 1 fixture, plus cases for shorter/longer/same, element 0,
      the last element, and a single-element list. **All seven are in
      `p121f-string-set-readback-rt`**, which reports the first mismatching index
      rather than a count. Output is **identical before and after the change** —
      the old path was correct and slow, so an unchanged result is exactly right,
      and it is what makes the codegen tests necessary (a green fixture cannot
      tell which path ran).
- [x] Tests: a case that overflows `dataCapacity` mid-loop, proving the grow path
      still works and the tight copy still normalizes. **Present** — 6 passes over
      60 elements, each pass longer than the last, so the region overflows
      **repeatedly** rather than once; plus a snapshot taken before the writes that
      must not observe them (the `copy_collection_tight` contract).
- [x] **ADDED: codegen inspection** (`tests/codegen_string_set_shift.rs`, 3 tests).
      Phase 2's coverage check asks for proof that *both* the fits-in-capacity and
      the overflow branches are covered, and no runtime fixture can supply it —
      the observable result is the same either way. These assert the widening
      copy, the narrowing copy, **both** offset fixups and the geometric grow are
      emitted, paired with a must-not-change proving a fixed-width `set` emits
      none of them.

Acceptance: **MET on the measurement, with two honest qualifications.**

Spike 2's variants, measured before (a compiler built from `56b368996`) and after,
2000 sets at each N:

| N | `same` | `longer` before → after | `shorter` before → after |
|---|---|---|---|
| 50 | 10 ns | 72 → **15** | 65 → **13** |
| 200 | 10 ns | 770 → **38** | 751 → **33** |
| 800 | 10 ns | 11240 → **335** | 10909 → **305** |
| 3200 | 10 ns | 125784 → **3408** | 126375 → **3087** |

**37× and 41× at N = 3200.** `same` is flat at ~10 ns throughout — the control,
showing the already-fast path was not disturbed.

*Qualification 1:* the residual growth is steeper than a clean O(N) because the
probe's mix changes with N, not because a super-linear cost remains — it does
2000 writes cycling `k MOD n`, so at n=50 only the first 50 widen and the other
1950 are same-length, while at n=3200 all 2000 widen. The per-set average
therefore blends two different operations in an N-dependent ratio. Reading a
growth exponent off it would be reading the probe, not the implementation.

*Qualification 2:* ~~golden drift confined to the Phase 1 set~~ — **it was not:
24 diffs across 5 fixtures, against a predicted 1.** The census asked the wrong
question; see Correction F2. Every one of the 24 is `.ncode` — no behavioural
golden moved anywhere — and regeneration touched exactly those 24 and nothing
else.

Also met: `scripts/artifact-gate.sh` = **1856 golden(s), 0 diff(s)** after
regeneration.
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

### F1 — the "existing geometric grow path" for a length-changing `set` did not exist

§3 said: *"Only a genuine `dataCapacity` overflow takes the existing geometric
grow path."* There was no such path. A length change fell back to the
`removeAt` + `insert` rebuild, and **that rebuild produces a TIGHT buffer** — so
the next widening overflowed again, and every one after it.

**Measurement is what caught it, and it would have been easy to declare victory
without.** With the in-block shift added but the overflow still falling back to
the rebuild:

| N | `longer` before | `longer` after shift-only |
|---|---|---|
| 50 | 72 ns | 72 ns |
| 200 | 770 ns | 828 ns |
| 800 | 11240 ns | 11619 ns |
| 3200 | 125784 ns | 122465 ns |

**Unchanged** — the shift never ran, because every widening overflowed on the
first call. Meanwhile `shorter` (which cannot overflow) improved ~7×, so a
narrower test would have shown a real win and hidden the fact that half the
feature was dead. This is the `.ai/collections.md` "dead work is invisible to
behavioral tests" pattern again: the code was correct, exercised, and had no
effect.

The fix is `emit_grow_list_data_capacity` — grow the data region **geometrically**
(reusing `emit_geometric_step`, as `append` does) and carry on shifting, so
reallocation happens O(log N) times instead of every call. It is deliberately
simpler than `append`'s grow: because `capacity` is unchanged, the header, the
whole entry table and the live data are one contiguous prefix, so it is a single
verbatim block copy and **no entry offset moves**.

With both halves:

| N | `longer` before | after | `shorter` before | after |
|---|---|---|---|---|
| 50 | 72 ns | **15** | 65 ns | **13** |
| 200 | 770 ns | **38** | 751 ns | **33** |
| 800 | 11240 ns | **335** | 10909 ns | **305** |
| 3200 | 125784 ns | **3408** | 126375 ns | **3087** |

**37× and 41× at N = 3200**, and `same` is unchanged at ~10 ns throughout, which
is the control: the fast path was not disturbed.

### F2 — the Phase 1 drift census under-counted, because it censused user source

Phase 1 recorded the expected `.ncode` drift set as **one** root
(`tests/byte-identity/collections`), by grepping fixture *sources* for
`List OF String` together with `collections::set`. The gate then reported **24
diffs across 5 fixtures**: `audio`, `collections`, `crypto`, `regex` (5 targets
each) and `rt-behavior/crypto/crypto-ec-valid` (4).

Not a bug — a census that asked the wrong question. `regex`'s fixture contains
neither string, so the drift cannot come from user code: **`.mfb`-bodied builtins
are monomorphized and emitted into the programs that use them**, so any fixture
pulling in a builtin whose body does a variable-width `set` re-emits that code.
The census should have asked "which fixtures emit a variable-width list `set`",
which includes emitted builtin bodies, not "which fixtures contain one in their
own source". This is `.ai/collections.md`'s "census a behavior by its effect"
rule, missed.

**Why the drift is nonetheless confirmed as only mine:**

1. **Every one of the 24 diffs is `.ncode`.** `crypto-ec-valid` carries 6 goldens
   and only its 4 `.ncode` moved; its `.run` did not. No behavioural golden moved
   anywhere in the run.
2. **Regeneration touched exactly 24 goldens** — `regen-ncodesum.sh` reported
   "141 refreshed, 0 missing" and `git status` shows precisely the 24 that
   diffed, with no extra churn.
3. **The fixtures that drift all pull in variable-width collection builtins**;
   `json`, `csv`, `http`, `io`, `math`, `net`, `money`, `datetime`, `encoding`,
   `fs`, `general` and `bits` do not drift.
4. The change is confined to `lower_list_set_in_place`'s variable-width branch
   plus two new helpers, and `a_fixed_width_set_does_not_emit_the_shift` pins
   that the fixed-width path emits none of it.

## Summary

The risk is entirely in entry-offset fixup, whose failure mode is silent garbage
past the modified element — which is why the tests read back every element rather
than folding a checksum. The O(N) shift itself is accepted as the floor; the win
is deleting the per-call allocation and the arena churn it causes, which
arithmetic in §2 puts at 371× → ~2–3× C without touching layout.
