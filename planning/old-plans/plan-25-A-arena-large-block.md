# plan-25-A: Arena large-block free-list (the benchmark master fix)

Last updated: 2026-07-05
Overall Effort: huge (>3d) — the whole plan-25 benchmark push (sub-plans A–E)
Effort: large (3h–1d)

## OUTCOME (2026-07-05, commit b13eec40) — DONE via a corrected diagnosis

Phase 1 (large-block bins) is IMPLEMENTED and correct. But the investigation
**overturned this plan's premise**: correctly-freed large churn was ALREADY flat
on baseline (`take` stays ~15ms after 2000 real frees). The 30× inflation is a
**temp-lifetime leak** — a bare call-result temporary like
`len(collections::transform(...))` was never freed until the enclosing FUNCTION
returned (no per-statement temp drop existed), so the arena bloated each hot loop
and unrelated ops walked a swollen free-list. The whole benchmark uses this
`len(collections::op(...))` idiom.

Fixed both: Phase 1 bins + a statement-scope temp-free pass (`pending_temp_frees`
in `lower_ops_inner`, registered in `lower_value`, claimed in `lower_value_owned`
and at move sites). Bare `String` temps are conservatively NOT freed (a string
call-result may be shared rodata — `toString(Boolean)` — or a borrowed view;
freeing one corrupts the arena). Records/unions/Results/collections ARE freed.

Result (release, --run 10): take 436→14, transform 199→8.6, partition 525→25,
reduceRight 552→31, removeAt 200→8.3, zip 181→8.6, mid 110→4.7, insert 200→10.5,
findIndex 232→22.5 (all −90%+). Full suite byte-identical runtime (0 build.log
diffs); 10 native-artifact goldens re-blessed. sortBy/window/flatten still high
(non-flat `List OF List` temps need recursive drop — future work).

**Phase 2 (bounded coalescing) — IMPLEMENTED as a large-bin flush-before-grow
drain, NOT boundary tags.** The boundary-tag design in this plan is unsound under
the "no live header" non-goal (a freed block can't tell whether its physical
neighbour is free without a marker on live blocks). Instead, before a large
request maps a fresh block, `arena_alloc` drains every large-block bin through the
audited address-ordered coalescing insert (`arena_alloc_large_flush`) and retries
the walk once (`flushed` guard) — address-adjacent large frees merge into a
big-enough run, recovering fragmentation. It drains ONLY the 64 large bins (the
measured pathology was draining the 128 *small* quick bins on a large grow), and,
because the temp-lifetime fix makes same-size large reuse a bin hit, it fires only
on a genuine miss. Validated: ~20k varying-size large allocations + a long-lived
list hold RSS at 2.5 MB with no crash.

**Refinement — statement-scope free restricted to flat COLLECTIONS only.** Freeing
record temps gave zero benchmark gain and regressed record-heavy hot loops
(vector math's ~1.6M `Float3` temps +55%, io read +63%) — small records park in
O(1) quick bins so their leak never caused the O(N²) blow-up that large list temps
did. Registering only `is_collection_type && is_freeable_flat_value` keeps every
list/parse/map win and removes the regressions; it is also the safest boundary
(a collection call-result is always a fresh, uniquely-owned arena block — never
rodata, never an argument alias).

Follow-up: free non-flat nested-collection temps (recursive drop glue) to recover
sortBy/window/flatten; audit string/record call-results if their temp frees are
ever wanted.

The single highest-leverage benchmark fix. Empirically, an mfb list operation
that costs ~15 ms in isolation costs **436 ms** in the full benchmark run — a
~30× inflation that appears only after the process has churned many *large* list
allocations. The cause is the arena allocator's **large-block free path**: freed
blocks above the quick-bin size class land on a list that is walked O(N) on every
subsequent alloc/free, so total allocation cost grows quadratically with churn
volume. Fixing this restores the isolated speed of ~15 list benchmarks at once
without touching a single collection op.

It complements:

- `./mfb spec memory arenas` (`src/docs/spec/memory/04_arenas.md` — the
  first-fit free-list and coalescing contract this plan changes).
- `planning/allocator-20-coalesce-size-authority.md` (the free-size-authority
  canary; this plan must keep its coalescing invariant intact).
- `planning/old-plans/plan-01-arena-update.md` (prior arena mixed-churn work —
  the "128 quick bins + designated-victim carve" fast path that this plan
  extends to large blocks).

## 1. Goal

- A list op's cost is independent of how much *unrelated* large-list allocation
  churn preceded it. Concretely: the repro below must keep `take` at ~15 ms
  after 2000 prior large-transform allocations (today it climbs 15 → 78 → 257 →
  673 ms).
- No change to any observable program result, golden `.ncode`, or allocation
  semantics — only the allocator's internal bookkeeping and time complexity.

### Non-goals (explicit constraints)

- **No per-allocation header.** Live allocations still carry no metadata
  (`plan-01-arena-update` non-goal, reaffirmed by `allocator-20`). Free-size
  remains compiler-supplied; this plan only reorganizes *free* bookkeeping.
- No change to `arena_alloc` / `arena_free` ABI, the RW-`__DATA` arena globals,
  thread-transfer arena hand-off, or the free-size-authority contract.
- No change to value/copy/move semantics or collection layout.

## 2. Current State

The arena free-list and coalescer live in `src/target/shared/code/*` runtime
helpers (`entry_and_arena.rs` — `arena_alloc`, `arena_free`,
`arena_insert_free`, first-fit walk `:1108-1198` region). The prior mixed-churn
fix (`plan-01-arena-update`) added:

- **128 quick bins** for small size classes (fast O(1) push/pop),
- a **designated-victim carve** and **small-gated flush** — but the "small-gated"
  qualifier means only *small* frees are routed to the fast structures.

Large frees (a 1000-element `List OF Integer` result is ~32 KB of entries + data)
fall through to the **general first-fit free-list**, which `arena_alloc` walks
linearly and `arena_insert_free` coalesces by adjacency. As large frees
accumulate, the list length grows and every alloc pays an O(N) walk → O(N²) total.

**Empirical proof** (release binary, 2026-07-05; repro in
`/tmp/iptp`, reproduced three ways):

| preceding churn                    | `take(base,500)`×500 |
|------------------------------------|---------------------:|
| fresh                              |  15 ms               |
| after 500 large (1000-elem) frees  |  78 ms               |
| after 1000 large frees             | 257 ms               |
| after 2000 large frees             | 673 ms               |
| after 8000 **small** (10-elem) frees | 53 ms (barely)     |

Superlinear in large-free count; **immune to small-free count** → the large-block
path is the culprit. Execution order confirms it: in `benchmark/mfb/src/main.mfb`
every list op *after* `test_list_flatten` (#17) is inflated (take 436, window 682,
sortBy 838, partition 525, reduceRight 552, zip 181, mid 109, insert 200,
removeAt 200, transform 199); ops *before* it (drop #12 = 15 ms) are not.

**Ground-truth confirmation via run-count scaling** (debug binary, benchmark.out,
2026-07-05). The reported median is the median of the per-run timings inside each
test's `FOR r = 0 TO run-1` loop. If each run were independent the median would be
flat in the run count; instead it grows **superlinearly**, because the arena
degrades across the runs (r=0 fast, r=9 slow), so a 10-sample median lands on a
more-degraded run than a 3-sample median:

| bench       | `--run 3` | `--run 10` | ratio (3.3× runs) |
|-------------|----------:|-----------:|------------------:|
| flatten     |    625    |    3644    | 5.8×              |
| take        |     67    |     436    | 6.5×              |
| partition   |     88    |     525    | 6.0×              |
| reduceRight |     96    |     552    | 5.7×              |
| findIndex   |     48    |     232    | 4.8×              |
| insert      |     31    |     200    | 6.5×              |
| window      |    295    |     682    | 2.3×              |

This is the signature of a per-allocation cost that rises with cumulative
large-block churn — i.e. the O(N) large-free walk. The fix must therefore make the
median **flat in the run count**, not merely smaller at one run count.

## 3. Design Overview

Give large frees the same amortized-O(1) treatment small frees already get, so
the general first-fit walk is never the steady-state path.

Two independent, separately-landable pieces:

1. **Segregated large-block bins (Phase 1).** Extend the quick-bin scheme to a
   set of *size-class* bins covering large sizes (power-of-two or fixed-stride
   classes up to a cap), so a large free is pushed onto its class bin and a large
   alloc pops from the smallest fitting class — O(1) amortized, no walk. Blocks
   above the top class (rare) keep the first-fit fallback.
2. **Bounded coalescing (Phase 2).** Keep adjacency coalescing (needed to avoid
   fragmentation and to preserve `allocator-20`'s size-authority invariant) but
   make it address-indexed (e.g. a sorted structure or boundary tags) so a free
   coalesces with neighbors in O(log N) / O(1) instead of an O(N) scan.

Correctness risk concentrates in Phase 2 (coalescing must still merge exactly the
adjacent free extents and never overlap a live block — the `allocator-20`
invariant). Phase 1 alone already removes most of the O(N²); Phase 2 recovers the
fragmentation behavior.

## 4. Detailed Design

- **Size classes.** Reuse the existing quick-bin constants
  (`error_constants.rs`, the `COLLECTION_GROW_*` / bin sizing region) and add a
  large-class table. A free of rounded size `s`: if `s ≤ small_cap` → existing
  quick bin; elif `s ≤ large_cap` → `large_bin[class_of(s)]`; else → first-fit
  fallback. Alloc mirrors it: try the exact/next class bin first, carve the
  remainder back to its own class bin.
- **Carve remainder re-parking.** The prior fix already re-parks post-flush
  remnants; extend that so a large-class carve remainder is re-binned by size
  rather than pushed to the general list.
- **Coalescing index.** Maintain free-block neighbors via boundary tags (a
  footer word in the *free* block only — allowed, since free blocks may carry
  metadata; live blocks still may not) so `arena_insert_free` finds `prev_end ==
  ptr` and `ptr+size == next` in O(1) without walking. This preserves
  `allocator-20`'s "no overlap with live" reasoning: boundary tags are written
  only when a block is freed and cleared when re-allocated.

## Layout / ABI Impact

- No change to **live** allocation layout or to any collection/record/union
  header → golden `.ncode` and all copy/transfer/thread paths unaffected.
- Free-block-only boundary tags are internal allocator state, invisible to
  compiled programs. `mfb spec memory arenas` (`04_arenas.md`) gains a paragraph
  describing the size-class bins and boundary-tag coalescing; the "frees are
  compiler-sized" contract is unchanged.

## Phases

### Phase 1 — Segregated large-block bins

Removes the O(N) large-free walk from the steady state; expected to recover the
bulk of the 30× inflation on its own.

- [x] Add a large-size-class bin table and `class_of(size)` mapping alongside the
      existing quick bins (`entry_and_arena.rs` free path; sizing constants in
      `error_constants.rs`). — DONE: 64 hashed `largeBin` (index = (size>>4)&63).
- [x] Route large frees to their class bin in `arena_free`; pop an exact-size
      match in `arena_alloc` before falling to first-fit.
- [x] Runtime proof: churn→`take` stays flat (fresh 14 → 2000-churn 13 ms).

Acceptance: MET — the poisoned-op table collapsed (take 436→14, etc.);
`scripts/test-accept.sh` green (0 build.log diffs; native-artifact goldens
re-blessed). Commit: b13eec40 (Phase 1) + this change.

### Phase 2 — Bounded coalescing (fragmentation recovery)

Restores adjacency coalescing without the O(N) scan, so long-running programs
don't fragment.

- [x] Coalesce large frees without an O(N) scan — DONE via
      `arena_alloc_large_flush`: a large grow drains the large bins through the
      audited coalescing insert and retries the walk once (NOT boundary tags,
      which the "no live header" non-goal forbids as unsound).
- [x] Stress test: ~20k varying-size large allocs + a long-lived list hold RSS
      at 2.5 MB with no crash (bounded fragmentation).

Acceptance: MET — mixed-churn stress shows bounded RSS; acceptance green.

## Validation Plan

- Runtime proof: `/tmp/iptp` churn→take repro (Phase 1) and mixed-churn stress
  (Phase 2); both must show flat per-alloc cost.
- Whole-benchmark proof: after A lands, re-run `./benchmark/run.sh 10` and
  confirm take/mid/insert/removeAt/partition/reduceRight/sortBy/window/zip/
  transform/findIndex/findLastIndex all drop toward their isolated cost.
- Doc sync: `src/docs/spec/memory/04_arenas.md`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Theorized gains (median, full-run)

These restore each poisoned op to near its isolated cost. Percentages are vs the
current `mfb-20260705-203627.log` medians.

| bench            | now (ms) | after A | Δ     |
|------------------|---------:|--------:|------:|
| take             |  435.9   |  ~18    | −96%  |
| window           |  682.2   |  ~30    | −96%  |
| sortBy           |  838.6   |  ~40    | −95%  |
| reduceRight      |  552.3   |  ~20    | −96%  |
| partition        |  525.5   |  ~25    | −95%  |
| findIndex        |  232.0   |  ~20    | −91%  |
| findLastIndex    |  230.3   |  ~20    | −91%  |
| transform        |  199.1   |  ~15    | −92%  |
| insert           |  200.2   |  ~30    | −85%  |
| removeAt         |  199.7   |  ~30    | −85%  |
| zip              |  181.4   |  ~20    | −89%  |
| mid              |  109.6   |  ~12    | −89%  |
| replace          |   46.4   |  ~10    | −78%  |
| chunks           |   30.4   |  ~10    | −67%  |
| filter           |   23.1   |  ~10    | −57%  |
| groupby          |    3.90  |  ~1.5   | −62%  |
| all / any        |  8.5/9.0 |  ~5     | −40%  |
| bignum modmul    |  233.8   | ~205    | −12%  |
| bignum modexp    |  128.2   | ~112    | −12%  |
| map lookup       |    6.14  |  ~5.0   | −19%  |

(bignum/map gains here are the allocation-churn component only; their algorithmic
fixes are in sub-plans C and D.)

## Summary

The engineering risk is entirely in Phase 2's coalescer correctness; Phase 1 is a
mechanical extension of the existing quick-bin fast path to large size classes and
delivers most of the win. Nothing about live-object layout, ABI, or program
semantics changes — this is pure allocator internals.
