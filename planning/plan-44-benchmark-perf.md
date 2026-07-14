# plan-44: Benchmark performance — close the gap to C/Python

Last updated: 2026-07-14
Effort: xlarge (multi-day; many independently-landable `plan-44-<letter>` sub-plans)
Platform under test: **aarch64 / macOS** (the target these logs were taken on)

Source logs (one matched timestamp `20260714-114214`): `benchmark/mfb-<ts>.log`,
`benchmark/c-O0-<ts>.log`, `benchmark/c-O2-<ts>.log`, `benchmark/python-<ts>.log`.
Startup is excluded (every workload is timed internally with
`datetime::monotonicNanos()` inside `test_*`); **median** is the metric. Re-measure
any fix with the **same `--run`** count as these logs.

This is the master plan + Task-1 ordered priority list for the benchmark
performance push. It scores every row in the current logs against the goals, orders
the work, and indexes the fix sub-plans (Task 2). The coverage plan is a separate
document, `planning/plan-45-benchmark-coverage.md`.

This is a **full fresh round** of research — every root cause below was re-derived
from the current tree at `file:line`, not carried forward from plan-39. Where plan-39
claimed a fix, its current state was re-verified in the source (all landed fixes are
confirmed still present and firing; no regressions found — see "Movement" below).

## The goals (priority order)

A benchmark's **priority = the first goal it fails**. Work lowest-numbered failures
first.

1. **G1** — mfb (MED) **< python** (MED).
2. **G2** — mfb ≤ c-O0 + **10 ms**.
3. **G3** — mfb ≤ c-O0 + **5 ms**.
4. **G4** — mfb ≤ c-O2 + **5 ms**.

**Override:** any mfb MED **≤ 5 ms is already complete**, regardless of G1–G4
(measurement noise). A benchmark is otherwise complete only when it beats all four.
Rows with **no cross-language baseline** (`Fixed`-typed / mfb-only) are excluded from
G1–G4 scoring; regression-track only.

## Scorecard summary

| Bucket | Count | Meaning |
|--------|------:|---------|
| **P1** (fails G1, loses to Python) | 29 | highest priority |
| **P2** (fails G2, > c-O0 + 10 ms) | 17 | |
| **P3** (fails G3, > c-O0 + 5 ms) | 6 | |
| **P4** (fails G4, > c-O2 + 5 ms) | 3 | lowest priority |
| no-baseline (Fixed, mfb-only) | 2 | excluded from scoring |
| complete (passes all 4, or ≤ 5 ms) | 57 | done |

Total scored fail-set = **55** non-complete benchmarks across P1–P4 (114 rows total).

**Movement since plan-39** (its scorecard predates the plan-40 coverage rows, so the
row set grew):

- **Now complete** (were offenders in plan-39): `io write` 26.7→2.0 (F1 buffered),
  `list take/drop/zip/findIndex/findLastIndex/reduceRight/any/all` (A2/A4), `string
  search` (now beats all four).
- **Now beat Python** (P1→P2/P3): `bignum modmul` 228→22, `modexp` 123→12 (D2 in-place
  `bnMod` — **verified present**, `benchmark/mfb/src/main.mfb:694-744`).
- **Improved, still an offender:** `fib` 108→79 (I1 elision verified), `window`
  203→117 (A4 native slice verified firing), `sortBy` 647→69 (A1 merge sort verified),
  `case` 155→67 (E1 ASCII path verified firing), `csv` 20→8.4 (G scalar cursor
  verified), `vector int` 99→57 (C1/C2 verified), `partition` 18→14, `chunks` 30→15,
  invtrig `atan/asin/acos/atan2` (B4 verified), `thread sum` 51.7→40.6.
- **New rows now measured** (plan-40 coverage), mostly new P1s that dominate the top of
  the list: `listchurn nested` (322, new worst), `mapchurn churn` (170), `regexbench
  replace/capture/alternation/compile`, `scalarbench classify/transform/listchurn`,
  `mapchurn iterate`, `strbuild splitjoin`, `io format`.
- **No regressions found.** Every plan-39 landed fix (A1/A2/A4, B2/B3/B4, C1/C2, D2,
  E1/E2, F1, G-csv, I1, K2) was re-verified present in the current tree.

---

## Task 1 — ordered priority list

Within each band, worst-first by mfb median. `Δpy`/`ΔO0`/`ΔO2` are `mfb − baseline`
(ms). **Sub-plan** maps each row to its fix (Task 2).

### P1 — loses to Python (fails G1) — do these first

| # | group/bench | mfb | py | Δpy | Sub-plan |
|--:|-------------|----:|---:|----:|----------|
| 1 | listchurn **nested** | 322.19 | 10.34 | +311.85 | **B** groupBy + **C** list-of-list |
| 2 | mapchurn **churn** | 170.16 | 1.21 | +168.94 | **A** in-place removeKey |
| 3 | regexbench **replace** | 139.43 | 0.03 | +139.39 | **E** regex matcher |
| 4 | list **window** | 116.73 | 8.64 | +108.08 | **C** list-of-list slice |
| 5 | list **sortBy** | 68.55 | 3.73 | +64.82 | **B** native source-generic |
| 6 | regexbench **capture** | 67.01 | 0.04 | +66.97 | **E** |
| 7 | string **case** | 66.73 | 28.26 | +38.47 | **G** string bulk/single-pass |
| 8 | list **copy** | 33.57 | 2.35 | +31.22 | **D** COW value-semantics |
| 9 | scalarbench **transform** | 32.28 | 1.28 | +31.00 | **J** arena churn |
| 10 | scalarbench **classify** | 28.45 | 14.52 | +13.93 | **I** scalar category |
| 11 | scalarbench **listchurn** | 26.44 | 9.50 | +16.93 | **J** arena churn |
| 12 | mapchurn **iterate** | 26.35 | 7.63 | +18.72 | **B** native merge/mapValues |
| 13 | regexbench **alternation** | 23.98 | 0.02 | +23.97 | **E** |
| 14 | liststr **hof** | 16.18 | 2.99 | +13.19 | **B** + **G** (reduce concat) |
| 15 | parse **regex** | 16.10 | 0.02 | +16.09 | **E** |
| 16 | list **chunks** | 15.24 | 1.68 | +13.56 | **C** |
| 17 | list **partition** | 14.14 | 7.18 | +6.96 | **B** |
| 18 | list **flatten** | 12.42 | 2.69 | +9.74 | **C** + **D** |
| 19 | strbuild **splitjoin** | 11.36 | 6.65 | +4.71 | **G** |
| 20 | list **removeAt** | 9.83 | 0.09 | +9.74 | **A** in-place removeAt |
| 21 | list **insert** | 9.54 | 0.21 | +9.33 | **A** in-place insert |
| 22 | io **format** | 8.72 | 7.04 | +1.68 | **G**/**J** (marginal) |
| 23 | parse **csv** | 8.37 | 0.74 | +7.63 | **E** (residual only) |
| 24 | listchurn **prepend** | 7.63 | 1.22 | +6.41 | *inherent O(n) front-shift* |
| 25 | liststr **build** | 6.91 | 0.14 | +6.77 | **A** + **G** |
| 26 | regexbench **compile** | 6.49 | 0.01 | +6.48 | **E** (compile-per-call) |
| 27 | map **str_ops** | 6.22 | 2.89 | +3.33 | **A** + **B** |
| 28 | parse **json** | 5.99 | 0.24 | +5.75 | **E** scalar cursor |
| 29 | map **int_ops** | 5.72 | 1.81 | +3.91 | **A** + **B** |

### P2 — > c-O0 + 10 ms (fails G2)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | math **simd** | 93.21 | 9.85 | +83.36 | **K** (capped) |
| 2 | math **pow** | 90.68 | 18.34 | +72.34 | **K** (capped) |
| 3 | recurse **fib** | 79.45 | 48.27 | +31.18 | **L** (capped) |
| 4 | math **tan** | 72.79 | 9.41 | +63.37 | **K** (capped) |
| 5 | vector **int** | 56.89 | 6.75 | +50.14 | **F** vector inline |
| 6 | vector **math** | 44.82 | 4.60 | +40.21 | **F** |
| 7 | thread **sum** | 40.63 | 9.38 | +31.25 | **L** (capped) |
| 8 | string **slice** | 37.45 | 23.27 | +14.18 | **G** |
| 9 | math **log10** | 37.10 | 7.90 | +29.20 | **K** (capped) |
| 10 | math **log** | 35.47 | 7.75 | +27.71 | **K** (capped) |
| 11 | math **sin** | 32.38 | 8.11 | +24.27 | **K** (capped) |
| 12 | math **cos** | 32.36 | 7.81 | +24.55 | **K** (capped) |
| 13 | vector **float** | 26.97 | 6.27 | +20.70 | **F** |
| 14 | bignum **modmul** | 22.01 | 5.11 | +16.90 | **H** limb reduction |
| 15 | math **acos** | 21.97 | 8.86 | +13.11 | **K** (capped) |
| 16 | math **exp** | 21.07 | 7.92 | +13.16 | **K** (capped) |
| 17 | math **asin** | 21.03 | 10.09 | +10.94 | **K** (capped) |

### P3 — > c-O0 + 5 ms (fails G3)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | math **atan2** | 22.58 | 14.04 | +8.54 | **K** (capped) |
| 2 | float **nbody** | 19.50 | 12.19 | +7.30 | **K** (finiteness-check) |
| 3 | math **atan** | 17.10 | 8.06 | +9.04 | **K** (capped) |
| 4 | bignum **modexp** | 12.18 | 2.82 | +9.36 | **H** |
| 5 | liststr **query** | 10.96 | 1.92 | +9.04 | **B** + **F** |
| 6 | float **leibniz** | 9.88 | 3.72 | +6.15 | **K** (finiteness-check) |

### P4 — > c-O2 + 5 ms (fails G4)

| # | group/bench | mfb | c-O2 | ΔO2 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | float **mandelbrot** | 53.76 | 19.92 | +33.84 | **K** (beats c-O0; c-O2 vectorizes) |
| 2 | math **sqrt** | 9.81 | 1.81 | +8.00 | **K** (B2 landed; ~optimal) |
| 3 | bits **ops** | 6.54 | 0.61 | +5.93 | **M** register-fusion |

### Excluded / already complete

- **No baseline (Fixed, mfb-only):** `math fixed` (29.45), `vector fixed` (13.38). Not
  scored; regression-track only. **F** (vector inline) lifts `vector fixed`
  incidentally.
- **Complete (passes all 4, or ≤ 5 ms):** 57 rows incl. `string search`, `recurse
  ackermann`, `math float`/`int`, `io write/read/buf_on/buf_off`, `list distinct/
  reduce/filter/take/drop/zip/findIndex/…`, `map set/lookup`, `mathpipe dft/stats`,
  `mathpipe finance` (mfb-only, 4.77), `arena transient/mixed/growshrink`, `primes`.

---

## Task 2 — fix sub-plans (index)

Grouped by **shared root cause** so one fix retires many benchmarks. Ordered by
aggregate priority reach. Each gets its own `plan-44-<letter>-*.md` if large enough to
split.

| Sub-plan | Covers (benchmarks) | Priority reach | Root cause (see body) |
|----------|---------------------|----------------|------------------------|
| **A** in-place mutation (insert/removeAt/removeKey) | list insert, removeAt; mapchurn churn; liststr build, map int/str_ops (partial) | 3×P1 + 2×P1 partial | mutation ops **alloc a fresh backing store + full O(n) copy/rebuild** per call — no in-place shift/tombstone path |
| **B** native/single-pass source generics | listchurn nested, sortBy, mapchurn iterate, partition, liststr hof/query, map int/str_ops | 6×P1 + 1×P3 | `merge`/`mapValues`/`groupBy`/`sortBy`/`partition` are `.mfb` source generics that re-materialize whole collections + per-element indirect calls |
| **C** list-of-list slice materialization | window, chunks, flatten, listchurn nested (build) | 4×P1 | native slice landed, but each piece is **separately alloc'd then bulk-append-copied then freed** — per-piece arena churn |
| **D** COW value-semantic collection buffers | list copy; amplifies flatten/window/nested/merge | 1×P1 + broad | whole-collection deep copy on every owner-boundary alias — no copy-on-write/refcount |
| **E** json/regex source packages | parse json/regex, regexbench compile/capture/alternation/replace | 6×P1 | json grapheme-materialize + per-char concat; regex CPS restart-per-position + caps-list copy per group + per-step continuation alloc |
| **F** vector Integer/Fixed op-inlining | vector int/math/float | 3×P2 | `length`/`distance`/`normalize` inline **Float-only**; Integer/Fixed pay FUNC call + per-op arena-block materialization + software isqrt |
| **G** string bulk-copy / single-pass | string case/slice, strbuild splitjoin, liststr hof, io format | 2×P1 + 1×P2 | two-pass per-op UTF-8 decode + byte-at-a-time scan/copy (no memchr/word-copy) + per-op arena string churn |
| **H** bignum limb-wise reduction | bignum modmul, modexp | 1×P2 + 1×P3 | D2 (in-place buffer) landed; residual is **bit-serial O(nbits×limbs)** via `collections::get`/`set` (~3.5M/run) — D1 (Barrett) untaken |
| **I** scalar classification integer-category | scalarbench classify | 1×P1 | `__strings_genCat` is a **4099-`IF` linear scan** returning a String, called 5× per scalar with String compares |
| **J** arena mixed-transient-churn quadratic | scalarbench transform/listchurn, io format (partial); **gates plan-45 arena rows** | 2×P1 | runtime arena free list still degrades quadratically under mixed-size transient churn (known-open bug) |
| **K** transcendental + float kernels | math sin/cos/tan/exp/log/log10/pow/simd/asin/acos/atan/atan2/sqrt; float leibniz/nbody/mandelbrot | 11×P2 + 3×P3 + 2×P4 | **structurally capped** by the double-double precision contract; B1/B2/B3/B4 exhausted |
| **L** integer overflow-check + call-tag residual | recurse fib, thread sum | 2×P2 | **structurally capped** — the accumulator/sum add can genuinely overflow → op stays fallible → mandatory check + per-call tag propagation |
| **M** bits register-resident operand fusion | bits ops | 1×P4 | every bits op spills+reloads both operands through stack slots — no register fusion across nested calls |

> **Key findings that reshaped the grouping this round** (re-verified at `file:line`,
> NOT copied from plan-39):
> - **The biggest untouched lever is native in-place mutation (A).** `insert`/
>   `removeAt` (list) and `removeKey` (map) all **allocate a fresh collection and copy
>   every surviving element per call** — there is no in-place path
>   (`builder_collection_mutate.rs:93`/`:3035`/`:4078`). `mapchurn churn` (170 ms, worst
>   map) is dominated by this **plus** a second-order cost: the rebuilt map has
>   `BUCKETS_READY = 0` (`builder_collection_layout.rs:1319`), so the very next `set`
>   rebuilds the whole hash index O(n). Contained native codegen, huge reach.
> - **`listchurn nested` (322 ms, new worst) is groupBy, not flatten.** `__collections_groupBy`
>   (`collections_package.mfb:227-247`) allocates **two** whole `transform` lists then
>   does per-element hasKey+get+append+set-back-into-map. Grouped under **B**.
> - **plan-39's landed fixes are all present and correct** — A1 merge sort, A2 single-pass
>   predicates, A4 native slice, B2/B3/B4 math, C1/C2 vector, D2 bnMod in-place, E1/E2
>   unicode, G csv scalar-cursor, I1 overflow elision, K2 NEON popcount. The residual
>   slowness is a *different, deeper* layer in each case, not a regression.
> - **`scalar classify` (I) is a genuine algorithmic defect, not arena:** the shared
>   category table `__strings_genCat` is a 4099-statement linear `IF` scan returning a
>   *String* category, invoked 5× per scalar (700k calls/run). Fixable to an integer
>   category code computed once per scalar.
> - **Math (K) and fib/thread (L) are structurally capped** and cannot reach their
>   bands without breaking a semantic contract (dd precision / integer overflow-trap).
>   Documented as ceilings, not open work.
> - **The arena quadratic (J) is still open** and is the foundation the plan-45 coverage
>   arena rows gate on — its acceptance criterion is bumping those tiny rows to realistic
>   N and staying linear.
> Highest leverage: **A** (3 severe P1 + lifts maps, contained native), **B** (6 P1,
> mostly source edits), **E** (6 P1), **C** (4 P1).

### Sub-plan A — native in-place mutation (insert / removeAt / removeKey)

**Covers (5):** list insert (9.5, P1), list removeAt (9.8, P1), mapchurn churn (170,
P1); partially liststr build (6.9, P1), map int_ops/str_ops (P1).

**Mechanism.** Reassignment fast paths (`try_inplace_*_assign`,
`builder_inplace_assign.rs`) exist for `append`/bulk-append/`set`/`prepend`/`concat`
only. `insert`, `removeAt`, and map `removeKey` have **no in-place path** — each call
allocates a fresh collection and copies every surviving element.

**Root cause (file:line).**
- **list removeAt:** `lower_collection_remove_at`
  (`src/target/shared/code/builder_collection_mutate.rs:250`) → `lower_list_remove_at`
  (`:3035`, fresh `arena_alloc` at `:3069`) — full O(n) copy + free every call. Workload
  `benchmark/mfb/src/list.mfb:597-615` does `removeAt(x,0)` 1000×/run → O(n²).
- **list insert:** `lower_collection_insert` (`:93`) → `lower_list_insert_collection`
  (`:474`), same alloc + full copy + shift. Workload `list.mfb:507-522`, 1000 mid-inserts.
- **map removeKey:** `lower_map_remove_key` (`:4078`) scans all entries (`:4143-4185`),
  `arena_alloc`s a fresh map (`:4213-4239`), copies every retained entry
  (`:4266-4300`). **Second-order cost:** the fresh map sets `BUCKETS_READY = 0`
  (`builder_collection_layout.rs:1319-1324`), so the next `set`/`hasKey`/`get` falls to
  `_mfb_rt_map_probe` and rebuilds the whole bucket index O(n)
  (`builder_collection_query.rs:168-171`). Workload `mapchurn.mfb:38-68`: 4000
  add/removeKey cycles at ~500 live → 4000 × 2×O(500) entry copies.

**Fixes (semantics-preserving — value-semantics unchanged; only the aliasing case with
a uniquely-owned target is optimized).**
- **A1 (biggest):** `try_inplace_remove_key_assign` for `m = collections::removeKey(m, k)`
  — mark the slot deleted / compact in place and **preserve `BUCKETS_READY`** (update the
  index, don't invalidate it). Removes both the O(n) rebuild and the index-rebuild
  second-order cost. Clears mapchurn churn; lifts map int/str_ops.
- **A2:** `try_inplace_remove_at_assign` for `x = collections::removeAt(x, i)` — in-place
  memmove of the tail (mirror `lower_list_prepend_in_place`'s entry-shift loop), guarded
  by unique ownership + not-FOR-EACH.
- **A3:** `try_inplace_insert_assign` for `x = collections::insert(x, i, v)` — in-place
  reserve-and-shift.
- Order by ROI: A1 (map, biggest), A2/A3 (list). Correctness gate: map/list checksums
  unchanged + `scripts/artifact-gate.sh` (byte-deterministic 4-target).

### Sub-plan B — native / single-pass source generics

**Covers (7):** listchurn nested (322, P1 — groupBy dominant), list sortBy (68, P1),
mapchurn iterate (26, P1), list partition (14, P1), liststr hof (16, P1), liststr query
(11, P3), map int_ops/str_ops (P1). Retires the largest P1 cluster after A.

**Mechanism.** These generics live in `src/builtins/collections_package.mfb` and run at
interpreted-generic constant factors: per-element `collections::get`/`set` native calls
+ indirect FUNC-pointer calls for the mapper/predicate, and several re-materialize a
whole intermediate collection.

**Root cause (file:line).**
1. **groupBy — listchurn nested 322 ms.** `__collections_groupBy`
   (`collections_package.mfb:227-247`) allocates **two** full `transform` lists then, per
   element, hasKey-probe + `get` bucket + `append` + `set` bucket back into the map.
   Workload `listchurn.mfb:58-92` runs it 20×/run over a 4000-elem flatten result.
2. **sortBy — 68 ms.** `__collections_sortBy` (`:102-157`) — merge sort (A1 landed,
   O(n log n)) but every element per pass does `get`×2 + in-place `set`×2 as full native
   builtin calls over **parallel items+keys arrays**, doubling the per-element call count.
3. **merge/mapValues — mapchurn iterate 26 ms, map int/str_ops.** `__collections_merge`
   (`:328-336`) does `MUT result AS Map = a`, and because `a` is a `Local` the
   owner-copy rule (`builder_values.rs:173`) **deep-copies the entire base map**
   (`lower_value_owned` `:127-134`) even to add 10 keys, then rebuilds its index
   (`BUCKETS_READY=0`). `__collections_mapValues` (`:249-255`, H1 verified — iterates
   directly, no intermediate lists) still builds a fresh N-entry map via per-element
   source `set` + FUNC call.
4. **partition — 14 ms.** `__collections_partition` (`:338-353`) — single pass (A2
   landed) but dominated by per-element `get` + indirect predicate call over 200k iters.
5. **liststr query/hof.** `find`/`findIndex`/`all` + String `get` (each String get
   materializes a fresh copy, `builder_collection_layout.rs:1798`) + indirect predicate.

**Fixes (semantics-preserving — same results/order/stability; only call count drops).**
- **B1:** native `groupBy` — single pass that mutates buckets in place without the twin
  transform lists (biggest single win for the worst row).
- **B2:** native `merge` — when the result is uniquely owned, extend into the base map in
  place (no deep copy, preserve `BUCKETS_READY`); native `mapValues` in-place over the
  copied map. Shares A1's index-preservation.
- **B3:** native-lower `sortBy` (single fused key+item array with inlined get/set) and
  the `partition`/`find*` predicate loops (inline the comparator/predicate like
  zip/slice were natively lowered), removing the indirect-call-per-element overhead.
- Order: B1 (nested), B2 (maps), B3 (list). Gate: every list/map checksum unchanged +
  `scripts/artifact-gate.sh`. **Sequence after A** (shares the in-place map machinery).

### Sub-plan C — list-of-list slice materialization

**Covers (4):** list window (117, P1), list chunks (15, P1), list flatten (12, P1),
listchurn nested build phase (part of the 322).

**Mechanism.** plan-39 A4 native slice (`try_inline_slice_op`,
`builder_collection_queries.rs:867` → `lower_list_slice_range:895`) is **verified live
and firing** (window 203→117). It removed the inner per-element append-loop, but the
outer list-of-list materialization remains: each of ~991 windows/call is separately
`arena_alloc`'d, then bulk-append-copied into the result (`builder_inplace_assign.rs:99`),
then freed. Workload `list.mfb:708-724` = ~99,100 alloc+copy+free/run.

**Root cause (file:line).** `__collections_window`/`__collections_chunks`
(`collections_package.mfb:283-313`) build the outer `List OF List` by appending
freshly-sliced pieces — per-piece sub-alloc + double copy + arena free churn.
`__collections_flatten` (`:257-266`) borrows each inner list (`get`) then bulk-append-
copies it into the result (200×100 bulk appends/run).

**Fixes.**
- **C1:** build the outer list in **one reserved pass** — pre-size the outer `List OF
  List` and slice directly into its data region, or a fused windowing intrinsic that
  emits all pieces without the intermediate free churn.
- **C2:** for flatten, sum the total length first, reserve once, and copy each inner
  block into the reserved buffer (no geometric regrow).
- Gate: list checksums + `scripts/artifact-gate.sh`. **Also stresses J** (mixed-size
  transient churn); C reduces the alloc volume J must absorb.

### Sub-plan D — COW value-semantic collection buffers

**Covers (1 direct + broad):** list copy (33.6, P1); amplifies flatten, window,
listchurn nested, and merge's base-map copy.

**Mechanism / root cause.** Whenever an aliasing source (Local/Global/MemberAccess)
reaches an owner boundary, `lower_value_owned` (`builder_values.rs:116-134`) →
`copy_collection_tight` (`builder_collection_layout.rs:310`) does a fresh `arena_alloc` +
verbatim memcpy of the whole backing block (also copying inline String payloads). There
is **no copy-on-write / refcount** anywhere. `list copy` (`list.mfb:76-102`, `RETURN xs`
over 2000 calls) is 2000 full-list deep copies vs Python's shallow ref.

**Fixes.**
- **D1:** copy-on-write / refcounted collection backing buffers — a `RETURN xs` shares
  the buffer and only copies on the next mutation. Retires `list copy` outright and cuts
  every bulk-append-copy and merge-deep-copy site.
- **Risk/scope:** largest design change in the plan (touches the value model + every
  mutation path). Semantics must stay identical (observably value-copy). Gate hardest:
  full `tests/` (value-semantics fixtures) + `scripts/artifact-gate.sh`.
- **Recommendation:** land A/B/C first (they cut most copy volume with contained edits);
  take D only if `list copy` and the residual copy amplification still dominate.

### Sub-plan E — json/regex source packages

**Covers (6):** parse json (6.0, P1), parse regex (16, P1), regexbench compile (6.5),
capture (67), alternation (24), replace (139) — all P1. (csv is **done** — G scalar
cursor verified; residual 8.4 ms is per-scalar get/append, structural.)

**Mechanism / root cause (file:line).**
- **json:** `__json_parse` (`src/builtins/json_package.mfb:292`) materializes the whole
  ~25 KB input into a `List OF String` of one-grapheme strings via `strings::graphemes`
  (`:293`), then indexes it with `collections::get` + String `=` compares; number tokens
  recurse per digit with `current & ch` concat (`__json_collectNumber:597,616`). Same
  pattern csv was moved off of, not yet applied.
- **regex:** interpreted CPS backtracking matcher. `__regex_makeCtx`
  (`src/builtins/regex_package.mfb:228-235`) materializes **both** a `List OF String` and
  a `List OF Integer` of the input; `__regex_searchFrom` (`:804-814`) restarts the full
  matcher at every start position; `__regex_setCap` (`:674`) `collections::set`-**copies**
  the caps `List OF Integer` on every group open/close; the matcher allocates a
  continuation record per node step (`:685,728,739,766`); `findAll`/`replace`/`match`
  recompile the pattern per call (no compiled handle — `regexbench compile` pays a full
  parse per line).
- **replace worst (139 ms):** `__regex_replace` (`:1777-1810`) adds per-match
  `strings::mid` re-walks from the string start (`:1796,1803,1809`), `out = out & …`
  growth, and `__regex_expand` (`:1647`) re-scalarizing the replacement per match (`:1648`).

**Fixes (source-package, no language change).**
- **E1 (json):** replace the `strings::graphemes` materialization with the scalar cursor
  csv already uses (`encoding::utf32Encode` + index a `List OF Integer`), and accumulate
  number/string tokens in a scalar buffer flushed with `utf32Decode` (kill the per-char
  `& ch` concat).
- **E2 (regex, biggest reach):** drop caps threading for zero-capture-group patterns;
  MUT the caps buffer in place within a `tryAt` instead of `set`-copying; a literal /
  first-scalar pre-filter in `__regex_searchFrom` to cut O(n) restarts; a compiled-pattern
  handle so `findAll`/`replace`/`match` parse once.
- **E3 (replace):** carry a scalar cursor and append unmatched spans by range instead of
  per-match `strings::mid` re-walks.
- Gate: parse checksums `csv=6003000`, `json=5000`, `regex=200`, and the regexbench match
  counts. **regex is structurally the heaviest** — E gets it much closer but a source CPS
  matcher will not reach C POSIX-NFA / CPython-`re` speed; bound the expectation.

### Sub-plan F — vector Integer/Fixed op-inlining

**Covers (3):** vector int (57, P2), vector math (45, P2), vector float (27, P2). Also
lifts `vector fixed` (mfb-only).

**Mechanism / root cause (file:line).** The register-native carrier is element-type
agnostic, but op-inlining is gated by `vector_op_inlinable`
(`src/target/shared/code/builder_vector_inline.rs:95-102`): `scale`/`dot`/`cross(3D)`
inline for **all** element types (C1 landed), but `length`/`distance`/`lerp`/
`lerp_unclamped` inline **Float-only** (`:99`), and `normalize`/`angle`/`abs`/`min`/`max`/
`clamp_length`/`project`/`reject`/`reflect`/`slerp`/`perpendicular`/`rotate_2d`/`cross(2D/4D)`
inline for **no** element type. So for Integer/Fixed, the ~19 `vector::length(...)`
wrappers per iter (`vector.mfb:147-172`) each: (a) make an out-of-line
`#vector_length_integerN` FUNC call, (b) **materialize the register-native operand to a
fresh N×8 arena block** (`vector_value_as_block:171-206` → `emit_build_inlined_record`),
(c) run software `__vector_isqrtRound`/`isqrtFloor` (`vector_package.mfb:74-102` — C2
FSQRT-seeded, verified, but still a FUNC chain). Float's `length` inlines to a single
NEON FSQRT and never touches the arena — hence 8× faster. `vector math` (200k iters) is
dominated by `normalize` (never inlined, any type).

**Fixes (semantics-preserving).**
- **F1 (biggest):** extend the inline rewrite (`:95-102`) to Integer/Fixed for
  `length`/`distance`/`normalize` — inline the sum-of-squares + the FSQRT-seeded isqrt
  register-native, removing the FUNC call **and** the boundary materialization/alloc.
- **F2:** keep Integer/Fixed vectors register-native across an inlined op chain so
  `length(scale(a,b))` never materializes an intermediate block (mirror the Float chain).
- **F3:** inline `abs`/`min`/`max`/`clamp_length` (pure lane arithmetic) for all types.
- Gate: vector checksums + `scripts/artifact-gate.sh`.

### Sub-plan G — string bulk-copy / single-pass

**Covers:** string case (67, P1), string slice (37, P2), strbuild splitjoin (11, P1),
liststr hof (reduce concat), io format (partial).

**Mechanism / root cause (file:line).** E1 (ASCII case fast path,
`builder_strings_builtins.rs:507-514,593-600`) and E2 (NFC quick-check,
`:718-781`) are **verified present and firing** on these all-ASCII workloads — the
Unicode table is not the bottleneck. The residual is structural:
- **case:** every op is **two full passes** (count `:501-543` + write `:587-622`), each
  calling `emit_utf8_decode_next` (`unicode.rs:78`) per codepoint, plus ~11 arena
  allocations per iteration (8 members + toString + 2 concats) × 50k ≈ 550k allocs/run
  (`string.mfb:35-44`).
- **slice/splitjoin:** `lower_strings_split` (`:1554`) and `lower_strings_join`
  (`:1342`) scan/copy **byte-at-a-time** (no memchr/word-copy) + a per-call multi-KB List
  alloc (`strbuild.mfb:63`, split→100 fields + join, 2000×/run).
- **hof reduce:** `acc & s` left-fold is O(n²) string concat.

**Fixes.**
- **G1:** single-pass case for the pure-ASCII path (fuse count+write; bytes stay 1-wide),
  and a word-at-a-time (`memchr`-style) delimiter scan + word-copy in `split`/`join`.
- **G2:** a builder/rope accumulator for `reduce`'s `acc & s` (O(n) instead of O(n²)).
- **G3 (io format):** the limb Float formatter (`float_format.rs`) + 5 per-line concats
  are the +1.68 gap; reserve the line buffer once and format in place.
- Gate: string checksums + `scripts/artifact-gate.sh`.

### Sub-plan H — bignum limb-wise reduction

**Covers (2):** bignum modmul (22, P2), modexp (12, P3).

**Mechanism / root cause.** D2 (in-place `bnMod` buffer) is **verified landed**
(`benchmark/mfb/src/main.mfb:694-744`, one 11-limb buffer, no per-bit allocs). Residual:
`bnMod` is still **bit-serial O(nbits×limbs)** — ~500 bit iterations/reduction, each
doing ~35 `collections::get`/`set` limb ops (shift-in `:715-720`, `bnCmp :722`,
conditional subtract `:725-739`) → ~3.5M bounds-checked list accesses per modmul run vs C
indexing a stack `uint32_t[]`.

**Fixes (source-level `.mfb`, no language change).**
- **H1 (D1, biggest):** replace bit-serial `bnMod` with **Barrett or limb-wise**
  reduction — O(limbs²) limb-ops instead of O(nbits×limbs), removing ~28× the inner
  iterations. Result must be bit-identical (cross-check old vs new remainder).
- Gate: bignum checksum unchanged (modmul/modexp printed checksum).

### Sub-plan I — scalar classification integer-category

**Covers (1):** scalarbench classify (28, P1).

**Mechanism / root cause (file:line).** The five `is*` predicates
(`src/builtins/strings_package.mfb:47-77`) each independently call
`__strings_genCat(toInt(sc))` then do String-equality compares. `__strings_genCat` (a
copy of `__regex_genCat`, `src/builtins/regex_unicode.mfb:8`) is **4099 sequential
`IF cp <= N THEN RETURN "xx"` statements** — a linear scan returning a *String* category.
Workload `scalarbench.mfb:54-86`: 70 scalars × 2000 passes × 5 predicates = 700k genCat
calls + ~1.4M String compares/run.

**Fixes (semantics-preserving — same classification results; ASCII workload).**
- **I1:** compute the category **once** per scalar and share it across the five
  predicates; return an **integer category code** (not a String) so the predicate check is
  an integer compare, not String equality.
- **I2:** binary-search / range-table the category lookup (or an ASCII fast path for
  cp < 0x80) instead of the 4099-`IF` linear scan.
- Gate: the five classification counts checksum unchanged across mfb/c/python (ASCII).

### Sub-plan J — arena mixed-transient-churn quadratic (foundational)

**Covers (2 + gate):** scalarbench transform (32, P1), scalarbench listchurn (26, P1),
io format (partial). **Gates every plan-45 arena-sensitive row.**

**Mechanism / root cause.** The known-open runtime arena bug (memory:
`arena-transient-churn-quadratic`, README:104-123): the free list degrades quadratically
under mixed-size **transient** churn — the short-lived `List`/`String` temporaries that
`strings::toScalars`/`fromScalars`/`graphemes`/`toBytes`/`normalizeNfc` allocate. Signature
is the min 2.7 / max 125 variance on `scalarbench transform` (`scalarbench.mfb:96-131`):
each pass builds a fresh `List OF Scalar` + `fromScalars` re-materializes a String via
per-codepoint `out & __encoding_fromCodepoint(cp)` (`encoding_package.mfb:278-290`),
fragmenting the free list. `scalarbench transform/listchurn` are still authored **tiny**
(`TODO(plan-39-A)` — carry forward to `TODO(plan-44-J)`).

**Fixes.**
- **J1:** fix the arena free list to stay linear under mixed-size transient churn —
  best-fit large-bin with bounded split, or raise/rework the large-bin hashing so
  mixed-size churn doesn't walk collision chains + drain (`entry_and_arena.rs` large-bin
  path). This is the residual F6 plan-39 left as secondary; the coverage rows now make it
  measurable.
- **Acceptance criterion:** the plan-45 arena-gated rows (`arena` group, `scalarbench
  transform/listchurn`, `string unibig`, `io binary`) are **bumped from tiny to realistic
  N in the same commit that lands J** and must stay **linear** across the `--run` loop.
- Gate: those rows' checksums + linear scaling; `scripts/artifact-gate.sh` for any codegen
  touched.

### Sub-plan K — transcendental + float kernels (structurally capped)

**Covers (16):** math sin/cos/tan/exp/log/log10/pow/simd/asin/acos/atan/atan2 (P2/P3),
sqrt (P4); float leibniz/nbody (P3), mandelbrot (P4).

**Mechanism.** All `Float` math is native inline NEON f64 (no libm). Scalar path
`lower_simd_float_scalar` (`builder_simd_float_math.rs:468-504`); pow via `emit_pow_scalar`
(`builder_pow.rs:154+`). Workload 2M kernel calls/row (`math.mfb:24-30`).

**Root cause + ceiling (file:line).** Each kernel open-codes a **double-double
compensated Horner** (`emit_compensated_horner`, twoprod+twosum per coefficient ≈ 5× a
plain `fmla`) to meet the ≤1-ULP / no-libm contract (`builder_math.rs:60-70`). tan needs
two dd Horners + a dd-divide (`:1036,1017`); pow is a full dd-log ∘ dd-exp (heaviest);
simd is the union of all kernels 2-lane. **This is inherent to computing these functions
in software dd arithmetic** — c-O0 calls hand-tuned libm assembly. plan-39's B items are
**verified landed and are the achievable wins:** B2 sqrt d-native (`builder_math.rs:1145`,
now ~optimal — sqrt P4 residual is the per-call domain check + loop overhead c-O2
vectorizes), B3 lane-drop (`:419-449`), B4 invtrig scalar-segment branch (`:768-836`). **B1
(setup LICM) is a confirmed dead end** — fully implemented, empirically regressed on
AArch64 (the ~6-12 constant vregs spill; reload > re-broadcast), reverted with no residue.

The **pure-float rows** (leibniz/nbody/mandelbrot) are **not** dd-capped — no kernels.
Their overhead is the plan-17 per-observation-boundary finiteness check
(`emit_float_result_check_fp`, `builder_math.rs:1292-1323`) plus fdiv chains c-O2
autovectorizes. mandelbrot already **beats c-O0** (loses only to c-O2 vectorization).

**Fixes (the only semantics-preserving levers left, both bounded).**
- **K1:** coalesce/sink the per-boundary finiteness checks across a chain of arithmetic
  nodes in leibniz/nbody (one check per observation, not per op) — bounded payoff (the
  residual is largely c-O2 vectorization, not check overhead).
- **BLOCKED:** the transcendental band cannot reach G2 without dropping the dd precision
  contract or importing libm — both semantic changes, off the table. **Documented as a
  ceiling, not open work.** Track for regression only.
- Gate: `tools/math-kernels/runtime_ulp.py` + scalar-vs-array bit identity + all math
  checksums unchanged.

### Sub-plan L — integer overflow-check + call-tag residual (structurally capped)

**Covers (2):** recurse fib (79, P2), thread sum (41, P2).

**Root cause + ceiling (file:line).** I1 elision is **verified firing:** fib's `n-1`/`n-2`
lower to bare `sub` under the `n<2` guard (`builder_numeric.rs:890-909,1010-1016`);
thread's `i = i+1` elides under the `i < stop` strict-upper bound (`:979-982`). The
residual is **irreducible under integer-overflow-trap semantics:**
- **fib:** `fib(n-1) + fib(n-2)` **can genuinely overflow i64**, so it keeps
  `adds`+`b.vc` (`:984-989`) and `fib` stays **fallible** — every one of ~29.9M calls pays
  a post-call result-tag check + propagation (`builder_emit_helpers.rs:179-189`) plus
  arg spill/reload (`:39,66`).
- **thread sum:** `total = total + i` (local+local, unbounded) can overflow → mandatory
  check every one of 40M iterations.

**Fixes (bounded; the check itself cannot be removed).**
- **L1:** a checked-add fast-path that returns the value without the full
  `RESULT_TAG_REGISTER` round-trip when the caller immediately consumes it (cut per-call
  tag propagation) + register-pin loop-carried vars to drop the arg spill/reload. Bounded
  — closes some of fib's ~21 ms over G2, not all.
- **BLOCKED:** eliding the sum/accumulator check would break the overflow-trap contract —
  off the table. **Documented as a ceiling.** Track for regression.
- Gate: overflow-trap tests + fib/thread checksums unchanged + `scripts/artifact-gate.sh`.

### Sub-plan M — bits register-resident operand fusion

**Covers (1):** bits ops (6.5, P4 — beats Python 117×, only > c-O2 + 5 ms).

**Mechanism / root cause (file:line).** Every binary bits op routes through
`lower_bits_two_integers` (`builder_bits.rs:37-68`), which **spills both operands to fresh
stack slots and reloads them** per call (`store_u64` `:47,:57`; `load_u64` `:65-66`), so a
fused expression like `bxor(h, bor(x, sl(x,3)))` round-trips every intermediate through
memory. Shift ops carry a mandatory 0..63 range-check (`:120-125`). K2 NEON popcount is
**verified landed** but neutral (1/15 ops). The individual ops already match C's ALU work.

**Fixes.**
- **M1:** keep operands register-resident across nested bits calls (eliminate the
  `lower_bits_two_integers` spill/reload); fold the range-check where the shift amount is a
  provable constant < 64.
- Lowest priority in the plan (already crushes Python; within ~6 ms of c-O2). Gate: bits
  checksum + `scripts/artifact-gate.sh`.

## Validation Plan (all sub-plans)

- **Correctness first:** every fix produces identical observable output — the benchmark
  checksums on stderr (`csv=6003000`, `json=5000`, `regex=200`, plus each group's printed
  checksum: list/map/string/vector/bits/bignum/scalar/math/float/thread) **unchanged** —
  and passes `scripts/test-accept.sh` + `tests/`. No language/semantic/syntax/precision
  change; value-semantics and integer-overflow-trap semantics preserved.
- Re-measure the affected group with the **same `--run`** as the source logs
  (`20260714-114214`); confirm the row's band improved (ideally to complete).
- Codegen changes: `scripts/artifact-gate.sh` (byte-deterministic 4-target self-diff).
  Math changes: `tools/math-kernels/runtime_ulp.py`.

## Open Decisions

- **A (in-place mutation) is the highest-ROI untouched lever** — contained native codegen,
  retires 3 severe P1s and lifts the map rows. Recommend landing A first, then B (shares
  the in-place map/index machinery), then C/E. Decision: A → B → C → E, then G/F/H/I.
- **D (COW) is a large design change** — recommend deferring until A/B/C have cut the copy
  volume; take D only if `list copy` + residual copy amplification still dominate.
  Decision: defer D; reassess after A/B/C.
- **J (arena quadratic) is foundational and gates plan-45** — its acceptance is bumping the
  tiny arena rows to realistic N and staying linear. Recommend scheduling J alongside the
  coverage plan's Phase 2. Decision: land J before bumping the gated coverage rows.
- **K (math) and L (fib/thread) are structurally capped** — cannot reach their bands
  without breaking the dd-precision or overflow-trap contract. Recommend documenting the
  ceiling and tracking for regression only; land the bounded levers (K1 finiteness-check
  coalescing, L1 tag-round-trip cut) only if cheap. Decision: ceiling accepted; bounded
  levers optional, not gating.
- **E-regex has a structural floor** — a source CPS matcher will not reach C POSIX-NFA /
  CPython-`re` speed; E gets it much closer but bound the expectation. Decision: pursue
  E1/E2/E3 for the large constant-factor wins; do not gate on matching C/Python.
- **Fixed rows have no baseline** — `math fixed` (29.5), `vector fixed` (13.4); F lifts
  vector fixed incidentally. Track for regression only. Decision: agreed.
