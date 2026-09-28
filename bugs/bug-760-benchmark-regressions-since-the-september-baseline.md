# bug-760: benchmark regressions since the 2026-09-03 baseline — record-field groupBy/sortBy fall off the fast path, removeKey re-hashes every moved entry, Map grow reclaims every time, and -O1 register allocation evicts on nearly every instruction

Last updated: 2026-09-28
Effort: x-large (1d–3d) — four independent sub-issues (A–D), each medium–large
Severity: HIGH — the default `-O1` build lost up to 7.5× on float/Fixed loops and
hundreds-× on one collection pattern, silently (every checksum still matches)
Class: Other (performance regression)

Status: Open
Regression Test: to be added in Phase 1 — see each sub-issue

A full `./benchmark/run.sh 10` at `9c1ea5568` (logs `benchmark/*-20260928-055319.log`,
now the committed baseline, `af442d65a`) against the previous baseline (`3ccc68297`,
2026-09-03) shows mfb 23–26% faster overall (geomean new/base on `min` 0.718 at -O1,
0.694 at -O3, against a 0.93 machine-control drift in the untouched C and Python
columns), but 42 -O1 rows more than 1.25× **slower**. C and Python were flat or
faster on every row, so none of it is the host. Every checksum still matches
(363 keys, 0 mismatched), so the output is right and only the cost regressed — which
is why no test caught any of it.

Bisection splits the regressions into four independent causes:

| sub | rows (mfb -O1 `min`, baseline → HEAD) | first bad commit |
|---|---|---|
| **A** | `list (Record-Dynamic) groupby` 0.361 → 243.7 ms (675×); `list (Record-Fixed) groupby` 0.191 → 1.483 (7.8×); `list (Record-Dynamic) sortBy` 8.31 → 44.0 (5.3×) | `a082faf5a` plan-149-A2 place copy forwarding |
| **B1** | `map (Fixed/Dynamic/key-Fixed/key-Dynamic) removeKey` 7–11×; `set (Fixed/Dynamic) remove` 7–11×; `map intchurn` 1.6×; `mapchurn churn` 1.3× | `54db20195` bug-732 finding 2 |
| **B2** | `map listagg` 0.979 → 1.878 (1.9×) | `8e260af26` bug-733 reclaim before grow |
| **C** | `float nbody` 19.5 → 146.4 (7.5×; 5.6× at -O3); `math fixed` 29.3 → 55.7 (1.9×) and a whole-program eviction signature | `8b36c06e3` and `d34fdae20` (plan-161-B, committed as "plan-158-B") |
| **D** | -O1-dominant: `list * sort*` 2–3.7×, `list (Dynamic) find` 3.3×, `width ascii` 3.4×, `width mixed` 2.7×; `string unicode/unibig` ~2× at -O1 and -O2 | `d34fdae20` (+ `8b36c06e3` for sort, unicode) — C's mechanism |

**The single correct behavior a fix produces:** every row above returns to within
1.25× of its 2026-09-03 `min` on comparable hardware (or better), with identical
checksums, **without** reverting the correctness or cost-contract fixes that
introduced the regression (plan-149 place forwarding, bug-732's index-kept removal,
bug-733's bounded churn, plan-161-B's promotion). The two `Fixed`
checksum/timing shifts from the bug-615/bug-659/bug-654 accuracy fixes are
**intended** and are not part of this bug (see C, "not a regression").

References:

- `benchmark/baseline/README.md` — the current baseline records that it includes these
  regressions; a fixed build must show these rows improve against it.
- `.ai/collections.md` "In-place mutation" — the self-update cost contract (bug-652:
  a declining in-place arm is a bug); records the ~445× `.mfb` groupBy fallback.
- `mfb spec memory collections`, *Cost* — "A self-update costs the elements it adds,
  removes or moves … It never costs the size of the part of `x` it leaves alone."
- `planning/completed/plan-149-A-audit-and-local-owners.md`,
  `planning/completed/plan-161-B-stack-slot-promotion.md`,
  `bugs/completed/bug-732-self-update-arms-cost-collection-size.md`,
  `bugs/completed/bug-733-map-set-churn-grows-without-bound.md`,
  `bugs/completed/bug-740-brogue-dig-leaf-call-and-spill-overhead.md`.
- Found while comparing the benchmark against the baseline (2026-09-28 session);
  probes and bisect logs under `/tmp/b760-{A,B,C,D}-*` (not durable — the reproductions
  below are self-contained).

---

## Sub-issue A — `groupBy` / `sortBy` on a forwarded record field fall back to the `.mfb` body

### Failing reproduction

```
' /tmp/b760-A-probe/src/main.mfb (TYPEs + helpers from benchmark/mfb/src/list.mfb)
LET rec AS RecDyn = RecDyn[1, buildStrRange(2000), 2]
LET base AS List OF String = rec.xs
acc = acc + len(collections::groupBy(base, strLenKey, strIdentity))
```

`mfb build -O 1 <proj> && build/probe.out rd_groupby` — min of 3, µs:

| commit | rd_groupby | d_groupby (plain local) | rf_groupby | rd_sortby | d_sortby |
|---|---|---|---|---|---|
| 3ccc68297 | 570 | 509 | 345 | 8174 | 8134 |
| 99b11f282 (parent) | 492 | 435 | 208 | 9235 | 8952 |
| **a082faf5a** | **227637** | 441 | **1538** | **44844** | 8936 |
| 9c1ea5568 HEAD | 229098 | 442 | 1518 | 44713 | 8942 |

- Observed: 460× / 7.4× / 4.9× vs the parent.
- Expected: the record-field rows cost what the plain-local rows cost.
- Contrast: plain-local `groupBy`/`sortBy` (unchanged); Record-Fixed `sortBy`
  (the Integer item type skips the gate).

### Root cause

1. `src/optimizer/opt1/placefwd.rs:forward` (new in `a082faf5a`) deletes
   `LET base = rec.xs` and rewrites reads of `base` into `rec.xs`; the NIR argument
   changes from `Local base` to `MemberAccess{Local rec, xs}` (`mfb build -O 1 --nir`).
2. The native fast paths gate on the **shape** of `args[0]`:
   `src/codegen/builtins/collections/func_group_by.rs:group_by_fast_path` requires
   `reeval_safe`, which accepts only `Local | Const | Global | LocalRef`;
   `src/codegen/builtins/collections/func_sort_by.rs:sort_by_fast_path` has the same
   `source_reeval_safe` check (required for a String item). `MemberAccess` fails both.
3. The call falls back to the out-of-line `.mfb` bodies: `__collections_groupBy`
   does `get`/`append`/`set` of the whole bucket per element (O(n·bucket)),
   `__collections_sortBy` copies both lists every merge pass. In `--ncode` the probe
   function goes from 5149 ops (inline native groupBy) to 45 (a `bl` to
   `_mfb_ifn_collections_5FgroupBy…`).
4. The fallback was reachable before (`groupBy(rec.xs, …)` written directly took
   230–254 ms at the parent); the forward rewrites the benchmark's idiomatic
   `LET base = rec.xs` into that form.

Confirmation: adding `MemberAccess` to both gates at HEAD (throwaway worktree)
restored rd_groupby 440 µs, rf_groupby 205 µs, rd_sortby 8.9 ms, checksums identical.

### Blast radius

- `func_group_by.rs:group_by_fast_path`, `func_sort_by.rs:sort_by_fast_path` — fixed by A.
- `src/codegen/builtins/vector/builder_vector_inline.rs:is_reevaluation_safe` — already
  correct (a `MemberAccess` rooted at a safe target is safe); the model to share.
- Other fast paths checked (chunks, flatten, findLastIndex, mapValues, window, merge,
  zip, partition, sort) gate on type or constant args — unaffected.
- `Local`-only shape checks forwarded reads may now miss — **unverified**, audit in
  Phase 1: `src/codegen/engine/value/builder_values.rs:owned_param_bind_moves`,
  `src/codegen/engine/function/function_lowering.rs:mark_vector_escaping_value`,
  `src/codegen/engine/control/builder_control.rs` `owns_field_iterable`,
  `src/codegen/builtins/math/gen_math.rs` finite-observed walk (`MemberAccess => false`).
- STATE owners (`h.state.xs`, forwarded by plan-149-B2 `7bb8c5f71`) — same gate,
  audit in Phase 1.

### Fix design

Make both gates accept a `MemberAccess` chain rooted at `Local`/`Global`/`LocalRef`
(recursing like `is_reevaluation_safe`, ideally sharing it) — the same notion of
"place" `placefwd` uses. **Rejected:** a blanket `MemberAccess{..}` (a call-rooted
target is not re-evaluation safe); teaching `placefwd` to decline before fast-path
builtins (special-cases the optimizer around a codegen gate). Verify lowering a
variable-width inline field twice does not deep-copy it (the confirmation run
suggests not). The O(n·bucket) `.mfb` `groupBy` body stays reachable through a
call-valued argument — latent, out of scope here (it is a separate cost bug; file it
if the audit shows a user-reachable path the fix does not cover).

---

## Sub-issue B1 — `removeKey` / Set `remove` re-hash and re-probe every moved entry

### Failing reproduction

```
LET base = buildMapII(n)          ' Map OF Integer TO Integer, keys 0..n-1
MUT nums AS Map OF Integer TO Integer = base
FOR i = 0 TO k - 1
  nums = collections::removeKey(nums, i)
NEXT
```

`mfb build -O 1 <proj>` — min µs, 3ccc68297 → 9c1ea5568:

| case | good | HEAD |
|---|---|---|
| front, n=300 k=300 | 87 | 1020 (12×) |
| front, n=3000 k=3000 | 8600 | 105500 |
| front, n=30000 k=30 | 1600 | 24500 |
| back, n=3000 k=3000 | 4200 | 200 (faster) |

Bisect: parent `3bc7e42d2` → `54db20195`: front 940 → 12 180 µs, back 458 → 46 µs,
`intchurn` 26 → 67 µs, `churn` 20.8 → 31.8 ms, `listagg` unaffected. The benchmark's
flat "~1.03 ms" on three rows is the same 300-front-removal workload, not a floor.

- Expected: removing in insertion order (the FIFO/sliding-window shape of
  `intchurn`/`churn`) costs no more per moved entry than it did before — ideally
  O(1) plus the lookup.

### Root cause

`src/codegen/collection/map/map_mutate.rs:lower_map_remove_key_in_place` now finds the
key by probing the index (`emit_map_find_index` → `emit_map_probe`), then calls
`map_bucket_remove.rs:lower_map_bucket_delete_helper` and
`lower_map_compact_from_helper` from the removed index. For **each** moved entry,
`compact_from` copies it, re-hashes its key (`home_slot`: byte-at-a-time FNV plus an
`unsigned_divide`) and walks the probe chain to rewrite bucket `i+1` → `w+1`.
Removing index 0 moves every remaining entry, so a front-removal loop is O(n²) with a
~12× larger constant than the old plain 5-word shift (which only marked the index
not-ready). State/Record variants got *faster* only because they now reach the same
in-place path instead of rebuilding the owner.

### Blast radius

- Everything that compacts through `compact_from`:
  `builder_inplace_assign.rs:try_inplace_remove_key_assign`,
  `try_inplace_set_remove_assign`, `lower_field_remove_key`, `lower_field_set_remove`
  — fixed by B1.
- `builder_inplace_setmap.rs:try_inplace_difference_assign` and the
  symmetricDifference arm (`emit_claim_members` + `emit_compact_claimed`) — compact
  once per call; cost **unmeasured**, audit in Phase 1.

### Fix design

Stop re-hashing per moved entry. Preferred: leave the `REMOVED` entry as a tombstone
and compact lazily once dead entries exceed half the count (insertion order still
holds; removal is O(1) + lookup). Acceptable alternative: when many entries move, do
one linear pass over the `2*capacity` bucket slots decrementing every bucket value
above `from+1` (no hashing), or mark the index not-ready and let the next probe
rebuild it. **Must not** regress bug-732's back-removal win (4200 → 200 µs) or its
cost contract.

## Sub-issue B2 — every in-place Map/Set grow runs a full reclaim

Bisect (`listagg` probe: group N ints into a `Map OF Integer TO List OF Integer` and
append into buckets): `8e260af26` 952 → 1670 µs.

### Root cause

Every in-place grow calls `_mfb_rt_map_reclaim`
(`map_bucket_remove.rs:lower_map_reclaim_helper`, via
`map_mutate.rs:emit_map_reclaim_before_grow`). Each call sums live bytes (O(count))
and, when dead bytes exceed live, byte-copies every live payload, heap-sorts the
relocated values and marks the index not-ready (so the next `getOr` rebuilds it).
Appending to List values keeps relocating them, so it fires constantly. Which step
dominates is **unprofiled** — Phase 1 measures it.

### Blast radius

Every in-place grow: `map_mutate.rs` set value-grow and entry-grow,
`map_compact.rs:emit_map_reserve`.

### Fix design

Keep a running dead-byte counter in the map header so the grow check is O(1) and the
reclaim runs only when it pays (amortized); copy words, not bytes. **Must not** undo
bug-733's bound on churn growth.

---

## Sub-issue C — plan-161-B fills the register pool; spills evict the same register on nearly every instruction

### Failing reproduction

`/tmp/b760-C-probe`: `nbody()` is `benchmark/mfb/src/main.mfb:test_nbody` verbatim;
`fixedMath()` is `benchmark/mfb/src/math.mfb:test_math_fixed`'s loop.
`mfb build -O 1 <proj> && build/probe.out`, min of 5, 3 runs each:

| commit | nbody µs | fixed µs |
|---|---|---|
| 3ccc68297 | 18.5k | 28.6k |
| 49f619092 (parent of 8b36c06e3) | 18.8k | 38k |
| 8b36c06e3 | 75k | 33.7k |
| d34fdae20 | 74.7k | 102k |
| 9c1ea5568 HEAD | 75k | 95.5k |

Signature (`mfb build -O 1 --ncode benchmark/mfb`, counting a `ldr R,[off]`
immediately followed by `str R,[off]`): **103 hits in 31 functions at `3ccc68297`,
77 072 in 991 at HEAD** (worst: `test_width_churn` 2072, `test_arena_growshrink` 1314,
`test_string_unicode`/`unibig` ~500, `test_nbody` 324–403, `bnMod` 380,
`ld`/`lsd_groupby` 236–317, `test_math_fixed` 231, lambdas).

### Root cause

- Before `8b36c06e3`, any call inside a value's live range — and every Float/Fixed op
  has a checked-error path that can call `_mfb_make_error_result` /
  `_mfb_rt_park_error` — excluded caller-saved registers, so float values lived in
  d8–d15 and d0–d7/d16–d31 stayed free as spill scratch.
- `8b36c06e3` (`vreg_live_across_clobber`) correctly relaxes that at -O1, but
  `src/codegen/engine/regalloc/linear_scan.rs:run` assigns first-come
  (`allocatable.iter().find(usable)`) with no furthest-end / lowest-weight spill
  choice, so nbody's ~30 long-lived state values plus masses take every d register.
- Short temporaries then spill and find no free scratch, hitting the eviction arm
  ("Every register is live, so commandeer one"), whose victim is always the first
  allocatable register (d0 / x8) into the same evict slot. The loop becomes one serial
  store→load chain (`str_q d0,[sp,4752]; fabs_d d0,d14; str_q d0,[sp,1872];
  ldr_q d0,[sp,4752]; …`): evict stores in the nbody loop 0 → 371, `str_q` 123 → 550,
  frame 3520 → 4800 bytes.
- `d34fdae20` (`src/optimizer/opt2/promote.rs:promote`, promotion at -O1) does the same
  to the integer pool; Q32.32 `Fixed` lives in x registers with inlined kernels, so
  `fixedMath` `ldr_u64` 697 → 1933, `str_u64` 195 → 1274 (603 to one slot).

**Not a regression (intended):** `math fixed`'s 29 → 38 ms step between `53d5d0b10`
and `9e5f274c0` is the bug-615 accuracy work (per-kernel: `asin`/`acos`/`atan`/`atan2`
slower, `sin`/`cos`/`tan`/`exp`/`pow`/`log` faster; kernels alone ~36.5 → 40 ms,
~10%). Its checksum shift (`math_fixed` .096519 → .096513) is recorded in
`benchmark/baseline/README.md`.

### Blast radius

Every function whose simultaneous demand exceeds a register class at -O1 and above
(991 functions in the benchmark alone). Also a ~15% share of B1/B2 (measured at
`49f619092` vs `d34fdae20`) and a ~1.1× drift on plain `sortBy` (sub-issue A's probe).

### Fix design

Proper spill choice in `linear_scan.rs:run`: when the pool is full, spill the active
interval ending furthest away (or lowest use-density) rather than refusing the
newcomer, so loop temporaries stay in registers and long-lived state goes to memory;
reserve one or two scratch registers per class once a function has spills; never
restore-then-resave the same slot across adjacent instructions. **Rejected:**
reverting plan-161-B / `vreg_live_across_clobber` (loses bug-740's win and the
correct liveness model). Stopgap only if the allocator change slips: cap how many
long-lived values phase 1/2 place in caller-saved registers under high pressure.

---

## Sub-issue D — -O1-dominant list `sort*` / `find` and `width` regressions

**Same mechanism as C** (the same two commits), surfacing in built-ins expanded inline
into the caller. Tracked separately because its acceptance rows and the -O2 contrast
differ.

### Failing reproduction

```
' /tmp/b760-D-probe/src/main.mfb
LET base = mkScr(20000)            ' append (i*7919) MOD n
FOR k = 0 TO 49 : acc = acc + len(collections::sort(base)) : NEXT
LET sb = mkStr(1000)               ' "s" & toString(i)
FOR k = 0 TO 4999 : acc = acc + collections::find(sb, "s999") : NEXT
' w = 75 x "The quick brown fox jumps over the lazy dog 0123456789 "
FOR k = 0 TO 9999 : acc = acc + strings::displayWidth(w) : NEXT
```

`mfb build -O <n> <proj>`, ms at -O1 / -O2, 3 runs each (checksums identical throughout):

| commit | sort | find | width ascii |
|---|---|---|---|
| 3ccc68297 | 34 / 33 | 7 / 8 | 209 / 201 |
| 49f619092 | 36 / – | 8 / – | 220 / – |
| 8b36c06e3 | 99 / 100 | 7 / 6 | 214 / 202 |
| **d34fdae20** | **218** / 37 | **33** / 6 | **780** / 202 |
| 9c1ea5568 HEAD | 210–240 / 37 | 39–46 / 6 | 720–770 / 200 |

### Root cause

`sort`/`find`/`displayWidth` lower inline (`src/codegen/builtins/collections/func_sort.rs`,
`src/codegen/builtins/strings/func_display_width.rs`), so their loop speed is decided by
the caller's register allocation. `8b36c06e3` lets long-lived values take caller-saved
registers (sort ×2.8 at -O1 **and** -O2). `d34fdae20`'s `promote` (called from
`builder_registers.rs:run_register_allocation`; 420 slots promoted in the sort probe,
`-v`) turns every private frame slot into a long-lived vreg that fills the file; the
merge loop's temporaries then take `linear_scan.rs:run`'s "commandeer" arm
(`str x8,[sp,#1072]; ldr x8,[sp,#832]; …; ldr x8,[sp,#1072]`), growing the merge loop
43 → ~140 instructions. `promote.rs:promote`'s doc claim that spilling is "never worse
than the frame round-trip it replaces" is false: the spill lands on a different, hotter
value than the one promoted.

**Why -O2 recovers (d34fdae20 onward):** `opt2::optimize_mir` Level-2 copy propagation
(1312 fires) and DCE (1648) delete the promotion's copies before allocation, collapsing
the promoted ranges (10 copies left for coalescing at -O2 vs 551 at -O1). -O1 runs
neither. That this explains the -O2 recovery at `d34fdae20` but not the -O2 sort loss
at `8b36c06e3` is inferred, not proven pass-by-pass — confirm in Phase 1.

**`string unicode` / `string unibig` (~2× at -O1 and -O2): same cause.** Bisected at
-O2 (probe: the two `benchmark/mfb/src/string.mfb` bodies standalone; unicode 17.3 →
42.9 ms at -O2): first bad `8b36c06e3`, second step `d34fdae20`, nothing else. The
"at -O1 only" in `8b36c06e3`'s subject is misleading — its gate
`level_enabled(1)` (`src/optimizer/mod.rs`) is `row_level <= active_opt_level()`, so
it is on at every level ≥ 1. The loss is in the inline straight-line table lookups of
`src/codegen/builtins/strings/func_normalize_nfc.rs:lower_out` and the grapheme/property
lookups (`normalizeNfc` ×3.7, `graphemes*` ×1.5–1.8 per op; `toBytes`/`caseFold`
unchanged): eviction pairs 0 → 669 → 721 across `49f619092` → `8b36c06e3` →
`d34fdae20`. The benchmark's -O3 improvement on these rows is **not** reproduced by
the standalone probe (it shows -O3 ×1.2–1.6 slower); unexplained — re-measure after
the C fix. `_mfb_arena_alloc` itself carries 16 eviction pairs at -O2, so rows that
allocate heavily likely pay a share too (unmeasured).

### Blast radius

Every -O1 function whose live values outnumber a register class; most exposed are
inline-expanded built-ins with multi-word loop state in large frames — collections
`sort`/`sortBy`/`find`, `strings::displayWidth` and the pad-to-width callers
(`helper_pad_to_width.rs`, `func_pad_*`). Fixed with C.

### Fix design

C's spill-choice fix covers D. D's own acceptance: at minimum, when no register is
free, spill a promoted slot rather than a short temporary; alternatively run Level-2
copy propagation + DCE at -O1 whenever promotion fired, or reject a promotion that
lengthens a spilled interval. Fix or prove (with a hot-loop-in-many-locals test)
`promote`'s "never worse" doc claim.

---

## Goal

- Every row in the table above within 1.25× of its 2026-09-03 `min` (C: `nbody`
  ≤ 24 ms, `math fixed` ≤ 40 ms allowing the intended bug-615 share), with checksums
  identical to `benchmark/baseline/*.sums`.
- The `ldr R,[off]; str R,[off]` eviction signature in the benchmark's -O1 ncode back
  to the order of the pre-regression count (≤ ~200 hits).

### Non-goals (must NOT change)

- No reverting plan-149 place forwarding, bug-732 index-kept removal, bug-733 bounded
  churn, or plan-161-B promotion — each fixed a real bug; the fix is to their cost.
- No changing any benchmark row to dodge a regression (e.g. writing `groupBy(rec.xs,
  …)` differently, or removing from the back) — the rows measure idiomatic code.
- No checksum or observable-output change; no ABI/layout change beyond the map header
  counter B2 may add (internal, runtime-private).
- The bug-615/bug-659/bug-654 `Fixed` accuracy shifts stay.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] A: codegen test (model: `tests/codegen/codegen_place_forward.rs`) that
      `groupBy`/`sortBy` on a forwarded record field and STATE field at -O1 lower
      inline (no call to `_mfb_ifn_collections_5FgroupBy…` / `sortBy`). RED at HEAD.
- [ ] B1: runtime scaling test (model: `tests/runtime/rt_list_set_widening_linear.rs`,
      better-of-3, N vs 8N) for front-order `removeKey` and Set `remove` whose per-op
      cost bound fails at HEAD and passed at `3bc7e42d2`.
- [ ] B2: runtime or codegen test that an append-into-bucket loop does not run the
      O(count) reclaim scan on every grow.
- [ ] C: codegen test (model: `tests/codegen/codegen_stack_slot_promotion.rs`) bounding
      the eviction signature / evict-slot stores in an nbody-shaped -O1 loop against
      the `49f619092` count. RED at HEAD.
- [ ] D: codegen test that an inline `collections::sort` merge loop in a many-locals
      function at -O1 has no eviction brackets (RED at HEAD); bisect `string unicode`/
      `unibig` — done: same cause (see D).
- [ ] Complete every "unverified / audit in Phase 1" blast-radius item above with a
      verdict.

Acceptance: each new test fails at HEAD for the documented reason; the audit list has
a verdict per site.
Commit: —

### Phase 2 — the fixes (one commit per sub-issue)

- [ ] A: shared re-evaluation-safe place predicate used by `group_by_fast_path` and
      `sort_by_fast_path` (and any audit hit).
- [ ] B1: tombstone / linear-pass compaction in `compact_from` paths.
- [ ] B2: O(1) reclaim trigger with a header dead-byte counter; word copies.
- [ ] C: spill-choice + scratch reservation in `linear_scan.rs:run`.
- [ ] D: covered by C; plus fix or prove `promote`'s "never worse" doc claim.

Acceptance: Phase 1 tests pass; contrast cases unchanged; Non-goals intact.
Commit: —

### Phase 3 — regenerate + full validation

- [ ] Regenerate goldens the codegen changes shift (ncode/`.ncodesum`, artifact gate);
      diff and confirm deltas are only the intended allocation/lowering changes.
- [ ] Full suite (`.ai/compiler.md` Hard Completion Gate).
- [ ] `./benchmark/run.sh 10`: every row in the table within the Goal bound, 0 checksum
      mismatches; re-baseline `benchmark/baseline/` and drop the "includes bug-760"
      note from its README.

Acceptance: full suite green; benchmark Goal met.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 tests above, under `tests/codegen/` and `tests/runtime/`.
- Runtime proof: `./benchmark/run.sh 10` compared with `/tmp/benchcmp.py`-style
  per-row ratios against the 2026-09-03 numbers (`git show 3ccc68297:benchmark/baseline/mfb-O1.log`).
- Doc sync: `.ai/collections.md` (fast-path gate shape; removal/reclaim cost),
  `.ai/codegen-invariants.md` (spill choice), `mfb spec` collections *Cost* if the
  removal cost statement changes.
- Full suite: per `.ai/compiler.md`.

## Open Decisions

- B1 — tombstone + lazy compaction (recommended: O(1) removal, simplest cost story)
  vs. linear bucket-decrement pass (keeps dense layout, still O(n) per front removal).
- C — full furthest-end spilling (recommended) vs. a pressure cap on phase 1/2
  (smaller change, leaves the first-come allocator).

## Summary

Four independent regressions, all silent (checksums identical), all introduced by
fixes that were correct in behavior. The engineering risk concentrates in C (register
allocator — affects ~1000 functions and every golden that pins ncode) and B1 (map
layout invariants). A is a small gate fix with a confirmed repro. Nothing in the
language surface or output changes.
