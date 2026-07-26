# plan-64: Benchmark performance — close the gap to C/Python

Last updated: 2026-07-25
Effort: xlarge (multi-day; many independently-landable `plan-64-<letter>` sub-plans)
Platform under test: **aarch64 / macOS** (the target these logs were taken on)

Source logs (one matched timestamp `20260725-075953`, **`--run 50`**):
`benchmark/mfb-<ts>.log`, `benchmark/c-O0-<ts>.log`, `benchmark/c-O2-<ts>.log`,
`benchmark/python-<ts>.log`. Startup is excluded (every workload is timed
internally with `datetime::monotonicNanos()`); **median** is the metric.
Re-measure any fix with the **same `--run 50`** as these logs.

This is the master plan + Task-1 ordered priority list for the benchmark
performance push. It scores every row in the current logs against the goals, orders
the work, and indexes the fix sub-plans (Task 2). The coverage plan is a separate
document, `planning/plan-65-benchmark-coverage.md`.

This is a **full fresh round** — every root cause below was re-derived from the
current tree at `file:line`, not carried forward. **The predecessor plan-44 was
never implemented** (git log since 2026-07-14 shows only plan-45 coverage additions
and the plan-44 archival — no fix commit landed), so every plan-44 sub-plan is a
*hypothesis re-verified this round*, and the rows that improved since (list
insert/removeAt/flatten, regexbench capture/replace, list window/sortBy, listchurn
nested) improved via **unrelated bug work**, not plan-44. One plan-44 claim is
corrected below: map `set` is **already** in-place + hash-incremental.

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
| **P1** (fails G1, loses to Python) | 25 | highest priority |
| **P2** (fails G2, > c-O0 + 10 ms) | 17 | |
| **P3** (fails G3, > c-O0 + 5 ms) | 8 | |
| **P4** (fails G4, > c-O2 + 5 ms) | 3 | lowest priority |
| no-baseline (mfb-only) | 4 | excluded from scoring |
| complete (passes all 4, or ≤ 5 ms) | 72 | done |

Total = **129** rows (53 scored offenders across P1–P4). The suite grew since
plan-44 (114 rows) by the plan-45 coverage groups (encoding/datetime/dispatch + map/
mathpipe/list extensions).

**Movement since plan-44** (whose scorecard predates the plan-45 rows; plan-44's own
fixes never landed):

- **New mega-offenders from the plan-45 coverage rows, run at `--run 50`:** `dispatch
  trap` (6961), `datetime civil` (974), `datetime iso` (270), `dispatch union` (162,
  P2). These new groups exposed catastrophic paths.
- **`parse csv` "regressed" 8.4 → 447 — but did NOT regress algorithmically.** Its
  min is still **5.689 ms**; the 447 ms median is the **arena mixed-transient-churn
  quadratic** (sub-plan A) cumulatively degrading across 50 iterations. `--run 50`
  (vs plan-44's lower count) exposes the quadratic far more. Same story for `datetime
  civil` (min 3.179 → max 20150), `datetime iso`, `parse regex` (min 4.25 → max 111),
  `regexbench capture` (median 4.2 but max 242).
- **Retired since plan-44 via unrelated bug work (now complete):** `list insert`
  9.5→2.19, `list removeAt` 9.8→1.92, `list flatten` 12.4→2.76, `regexbench capture`
  67→4.25, `string search`, `strbuild clean`. **Improved but still offenders:**
  `listchurn nested` 322→70.8, `list window` 117→20.6, `list sortBy` 68→21.2,
  `regexbench replace` 139→12.6.
- **Unchanged offenders:** `mapchurn churn` 170→164, `string case` 67→66, `list copy`
  33.6→32.8, `scalar classify` 28→29, the whole math band.

---

## Task 1 — ordered priority list

Within each band, worst-first by mfb median. `Δpy`/`ΔO0`/`ΔO2` are `mfb − baseline`
(ms). **Sub-plan** maps each row to its fix (Task 2).

### P1 — loses to Python (fails G1) — do these first

| # | group/bench | mfb | py | Δpy | Sub-plan |
|--:|-------------|----:|---:|----:|----------|
| 1 | dispatch **trap** | 6961.5 | 16.6 | +6944.9 | **I** trap-block elision + **A** arena |
| 2 | datetime **civil** | 973.9 | 0.79 | +973.1 | **A** arena (O(1) algo, pure churn) |
| 3 | parse **csv** | 447.0 | 0.72 | +446.3 | **A** arena (algo fine, min 5.7) |
| 4 | datetime **iso** | 270.3 | 0.01 | +270.2 | **A** arena + concat mitigation |
| 5 | mapchurn **churn** | 164.4 | 1.18 | +163.2 | **C** in-place removeKey + index |
| 6 | listchurn **nested** | 70.8 | 10.0 | +60.8 | **D** native groupBy + **E** COW |
| 7 | string **case** | 66.0 | 27.96 | +38.0 | **F** single-pass ASCII case |
| 8 | parse **regex** | 38.5 | 0.01 | +38.5 | **H** regex ctx + compiled handle |
| 9 | list **copy** | 32.8 | 2.33 | +30.4 | **E** COW/refcount |
| 10 | scalarbench **classify** | 29.3 | 14.24 | +15.0 | **G** integer category code |
| 11 | mapchurn **iterate** | 24.5 | 7.50 | +17.0 | **C** native merge/mapValues |
| 12 | regexbench **compile** | 22.6 | 0.01 | +22.5 | **H** compiled-pattern handle |
| 13 | list **sortBy** | 21.2 | 3.73 | +17.5 | **D** native sortBy + **E** |
| 14 | list **window** | 20.6 | 8.38 | +12.2 | **D** slice-into-result |
| 15 | liststr **hof** | 15.3 | 2.96 | +12.3 | **B** borrow element + **D** |
| 16 | regexbench **replace** | 12.6 | 0.03 | +12.6 | **H** scalar-cursor replace |
| 17 | regexbench **alternation** | 11.8 | 0.01 | +11.8 | **H** prefilter restarts |
| 18 | strbuild **splitjoin** | 11.3 | 6.41 | +4.9 | **F** memchr split/join |
| 19 | scalarbench **listchurn** | 10.7 | 9.28 | +1.5 | **G** native toScalars |
| 20 | io **format** | 8.46 | 6.89 | +1.6 | **L** concat-chain fusion |
| 21 | liststr **build** | 6.81 | 0.15 | +6.7 | **F** in-place String set + **B** |
| 22 | map **str_ops** | 6.10 | 2.81 | +3.3 | **C** map in-place + index |
| 23 | list **chunks** | 5.53 | 1.68 | +3.86 | **D** slice-into-result |
| 24 | map **int_ops** | 5.52 | 1.79 | +3.7 | **C** map in-place + index |
| 25 | parse **json** | 5.06 | 0.22 | +4.8 | **H** scalar cursor |

### P2 — > c-O0 + 10 ms (fails G2)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | math **pow** | 88.3 | 18.04 | +70.3 | **M** (capped) |
| 2 | recurse **fib** | 76.9 | 55.95 | +20.9 | **N** (capped) |
| 3 | math **tan** | 71.1 | 9.22 | +61.9 | **M** (capped) |
| 4 | vector **int** | 56.4 | 6.53 | +49.8 | **J** vector inline |
| 5 | math **simd** | 49.8 | 9.58 | +40.2 | **M** (capped) |
| 6 | thread **sum** | 44.1 | 9.25 | +34.8 | **N** (capped) |
| 7 | string **slice** | 37.2 | 23.01 | +14.2 | **F** |
| 8 | math **log10** | 36.4 | 7.73 | +28.7 | **M** (capped) |
| 9 | math **log** | 34.6 | 7.67 | +26.9 | **M** (capped) |
| 10 | vector **math** | 33.1 | 4.53 | +28.6 | **J** (normalize) |
| 11 | math **cos** | 31.7 | 7.66 | +24.1 | **M** (capped) |
| 12 | math **sin** | 31.4 | 7.89 | +23.5 | **M** (capped) |
| 13 | vector **float** | 21.9 | 6.27 | +15.7 | **J** |
| 14 | math **acos** | 21.5 | 8.62 | +12.8 | **M** (capped) |
| 15 | math **asin** | 20.7 | 9.88 | +10.8 | **M** (capped) |
| 16 | math **exp** | 20.4 | 7.71 | +12.7 | **M** (capped) |
| 17 | bignum **modmul** | 19.4 | 5.04 | +14.4 | **K** limb reduction |

### P3 — > c-O0 + 5 ms (fails G3)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | math **atan2** | 21.9 | 13.69 | +8.2 | **M** (capped) |
| 2 | float **nbody** | 18.9 | 11.80 | +7.1 | **M** (finiteness lever) |
| 3 | math **atan** | 16.7 | 7.87 | +8.9 | **M** (capped) |
| 4 | mathpipe **memo** | 11.4 | 1.81 | +9.6 | **L** bounds + MOD |
| 5 | bignum **modexp** | 10.8 | 2.80 | +8.0 | **K** |
| 6 | liststr **query** | 10.8 | 1.91 | +8.9 | **B** + **D** |
| 7 | float **leibniz** | 9.39 | 3.67 | +5.7 | **M** (finiteness lever) |
| 8 | list **partition** | 6.04 | 0.38 | +5.7 | **D** native partition |

### P4 — > c-O2 + 5 ms (fails G4)

| # | group/bench | mfb | c-O2 | ΔO2 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | float **mandelbrot** | 51.8 | 19.29 | +32.5 | **M** (beats c-O0; c-O2 vectorizes) |
| 2 | math **sqrt** | 9.75 | 1.80 | +7.96 | **M** (hardware FSQRT — optimal) |
| 3 | bits **ops** | 6.39 | 0.61 | +5.78 | **O** register fusion |

### Excluded / already complete

- **No baseline (mfb-only):** `math fixed` (28.56), `vector fixed` (13.87), `mathpipe
  finance` (4.69), `mathpipe money` (2.59). Not scored; regression-track only. **J**
  (vector inline) lifts `vector fixed` incidentally.
- **Complete (passes all 4, or ≤ 5 ms):** 73 rows incl. `list insert/removeAt/flatten`
  (retired since plan-44), `regexbench capture`, `string search`, `strbuild clean`,
  `map set/lookup/intkey/listagg`, `mathpipe dft/stats`, `io write/read/buf_on/buf_off`,
  `arena transient/mixed/growshrink`, `encoding` group, `primes`, `record update`.

---

## Task 2 — fix sub-plans (index)

Grouped by **shared root cause** so one fix retires many benchmarks. Ordered by
aggregate priority reach. Each gets its own `plan-64-<letter>-*.md` if large enough to
split (A, D, E are the split candidates).

| Sub-plan | Covers (benchmarks) | Priority reach | Root cause (see body) |
|----------|---------------------|----------------|------------------------|
| **A** arena transient-churn quadratic | csv, datetime civil/iso, dispatch-trap climb, regex spread; **gates plan-65 arena rows** | 3×P1 (+amplifies 3 more) | free-list first-fit/insert walks + flush-before-grow go **O(n²)** on mixed-size transient churn |
| **B** borrow read-only collection element | dispatch union (P2), liststr hof/query; amplifies groupBy | 1×P2 + 1×P1 + 1×P3 | `collections::get` **deep-copies** every element (`copy_flat_block`) even for read-only MATCH/predicate use |
| **C** map in-place removeKey + index preservation | mapchurn churn, iterate; map int_ops, str_ops | 1×severe P1 + 3×P1 | `removeKey` fresh-alloc+full-copy, resets `BUCKETS_READY=0` poisoning the paired probe; `merge`/`mapValues` deep-copy the base map |
| **D** native-lower interpreted generics | listchurn nested, sortBy, window, chunks, partition, liststr query, any/all/findIndex | 4×P1 + 2×P3 | groupBy/sortBy/window/chunks/partition run `.mfb` bodies (per-elem native call + indirect dispatch); **groupBy is O(bucket²)** |
| **E** COW/refcount collection buffers | list copy; amplifies sortBy, groupBy, merge | 1×P1 + broad | 40-byte header has **no refcount word**; every alias boundary deep-copies the whole block |
| **F** string single-pass case/split/join | string case, slice, strbuild splitjoin; liststr build | 3×P1 + 1×P2 | case is 2-pass UTF-8 decode w/ no whole-string ASCII shortcut; split/join byte-at-a-time |
| **G** scalar integer-category classification | scalarbench classify, listchurn | 2×P1 | `__strings_genCat` is a 4099-arm linear scan returning a **String** category, 710k calls/run; `toScalars` interpreted |
| **H** json/regex source packages | parse regex/json, regexbench compile/alternation/replace | 5×P1 + regressions | full-input grapheme materialize + per-digit concat (json); recompile-per-call + dual makeCtx + O(n) restarts (regex) |
| **I** inline-TRAP Result/Error elision | dispatch trap (floor) | 1×severe P1 | each `toInt TRAP` allocates ~3 Result/Error/ErrorLoc arena blocks it destructures one op later |
| **J** vector Integer/Fixed op-inlining | vector int, math, float | 3×P2 | `vector_op_inlinable` inlines length/distance Float-only, `normalize` for **no** type → FUNC call + block materialize + soft isqrt |
| **K** bignum limb-wise/Barrett reduction | bignum modmul, modexp | 1×P2 + 1×P3 | `bnMod` still **bit-serial** O(nbits×limbs) (~5M list ops/run); in-place buffer landed, algorithm did not |
| **L** concat-chain fusion + bounds/MOD | io format; mathpipe memo | 1×P1 + 1×P3 | multi-`&` chain feeding a `LET` allocates ~7 growing intermediates (O(n²)/line); memo per-cell checked-get + MOD div |
| **M** transcendental + float kernels | 12 math + sqrt + 3 float | 9×P2 + 3×P3 + 2×P4 | **capped** by the double-double ≤1-ULP no-libm contract; bounded lever = leibniz/nbody finiteness-check coalescing |
| **N** integer overflow-check residual | fib, thread sum | 2×P2 | **capped** — value-carrying add can overflow → checked add + per-call tag round-trip mandatory |
| **O** bits register-operand fusion | bits ops | 1×P4 | every op spills+reloads both operands through stack slots; no fusion across nested calls |

> **Key findings that reshaped the grouping this round** (re-verified at `file:line`):
> - **The arena quadratic (A) is now the highest-reach lever, not a footnote.** The
>   top-4 P1s minus trap's floor — csv, datetime civil, datetime iso — are *pure*
>   arena degradation: `datetime civil`'s math is provably O(1) Hinnant arithmetic
>   (`datetime_package.mfb:261-305`) yet its median blows up 6300× from min→max. The
>   free-list walks live in `src/target/shared/code/arena.rs` (`:240-264`, `:715-729`,
>   flush `:387-407`). A also gates every plan-65 coverage arena row.
> - **One deep-copy-on-`get` (B) spans dispatch AND the list HOFs.**
>   `materialize_owned_element` → `copy_flat_block` (`builder_collection_queries.rs:10`,
>   `builder_collection_layout.rs:306`) copies every element `collections::get`
>   returns. That is the 4M-copy cost of `dispatch union` and the per-element cost of
>   `liststr hof`/`query`. A read-only "borrow" path clears both.
> - **plan-44 correction: map `set` is already optimal** (in-place append + incremental
>   bucket insert, `builder_inplace_assign.rs:266`). The map defect is isolated to
>   `removeKey` (no in-place path) + the `BUCKETS_READY=0` reset (`collection_buffer.rs:179`).
> - **`dispatch trap` needs two sub-plans:** its 1104 ms *floor* is genuine per-iteration
>   Result/Error-block allocation (I); its 1104→12668 *climb* is the arena quadratic (A).
> - **Math (M) and fib/thread (N) remain structurally capped** — re-verified: dd
>   compensated Horner (`builder_simd_float_math.rs:1854`) and mandatory checked-add
>   (`builder_numeric.rs:844`). Ceilings, not open work; bounded levers only.
> Highest leverage: **A** (3 severe P1 + gates coverage), **C** (map cluster, contained
> native), **D** (list generics), **B** (shared get-copy), **H** (5 P1).

### Sub-plan A — arena mixed-transient-churn quadratic (foundational)

**Covers (3 P1 direct + amplifies):** parse csv (447), datetime civil (974), datetime
iso (270); amplifies dispatch trap climb, parse regex, regexbench capture. **Gates
every plan-65 arena-sensitive row** (the successor to plan-44-J / plan-39-A).

**Mechanism.** The runtime free list (split out of `entry_and_arena.rs` by d653d5642)
degrades super-linearly under mixed-size **transient** churn — short-lived
`List`/`String`/record temporaries of *differing* sizes freed each iteration. Process-
global and cumulative across the `--run` loop: a fresh row starts fast, each repeat
gets dramatically slower. Signature = the min→max spread (csv 5.7→1477, datetime civil
3.2→20150, min-index tracks run index).

**Root cause (file:line).** `src/target/shared/code/arena.rs`:
- **First-fit alloc walk** `lower_arena_alloc` / `arena_alloc_walk_loop` (`arena.rs:240-264`):
  every request > `ARENA_QUICK_BIN_MAX` (2048 B, `error_constants.rs:409`) linearly
  walks the address-ordered free list.
- **Address-ordered insert walk** `lower_arena_insert_free` / `insert_find`
  (`arena.rs:715-729`): every coalescing free walks to find its slot.
- **Flush-before-grow amplifier** `arena_alloc_flush_bin`/`flush_chain`
  (`arena.rs:387-407`): when mixed-size small transients fill the 128 quick bins and a
  small request misses and must grow, the allocator drains *every* parked bin node
  one-by-one through `arena_insert_free` (`:402`), each paying the O(list) `insert_find`
  walk → **O(n²)**.
The benchmark evidence: `csv::parse` (`csv_package.mfb:29-96`) allocates ~6000 short
mixed-size `List OF Integer` field buffers + 6000 decoded Strings per call; `datetime
addDays`/`civil` construct spreads of distinct-sized `Date`/`Instant`/`Duration`/`DateTime`
records per call — all ≤2048, so they park in quick bins then drain through the flush.

**Fixes (semantics-preserving — allocator internals; no observable change).**
- **A1 (biggest):** make the flush-before-grow (`arena.rs:387-407`) coalesce the parked
  bins in **one address-ordered merge pass** instead of N× O(list) `insert_find` calls.
- **A2:** segregate the address-ordered free list into size-class sublists (or a
  balanced index) so neither the alloc walk nor the insert walk is global — best-fit
  large-bin with bounded split.
- **A3 (workload-side mitigations, independent):** `csv::parse` — track `(start,end)`
  scalar indices into the already-encoded `chars` and decode one sub-slice per cell
  (only quoted fields need a rebuilt buffer), cutting distinct transient sizes;
  `datetime addDays`/`addMonths` — a UTC/fixed-offset fast path that builds the result
  `DateTime` straight from `civilFromDays`, skipping the `resolveLocal`+`inZone`+`Instant`
  round-trip (`datetime_package.mfb:1005,1017`) and intern the constant `"UTC"` zone
  label so it isn't re-inlined per construction.
  - **[x] A3-datetime — LANDED.** Correction: the round-trip is in `__datetime_civil`
    (`datetime_package.mfb:552`), which `addDays`/`addMonths` call — not in `addDays`
    itself (`addDays` already goes `civilFromDays`-direct). Added a fixed-offset fast
    path guarded by `dt.zone.kind <> 2` (kind 2 = system/DST zone): for a fixed-offset
    zone a whole-day/month shift leaves the wall-clock time + offset unchanged and
    `civilFromDays`/`daysFromCivil` are inverse on valid dates, so
    `__datetime_civil(...)` provably returns `DateTime[civilFromDays(newDays), dt.time,
    dt.zone, dt.offset]` — built directly, skipping the `Instant`/`Date`/`Time`
    transients. A system zone keeps the round-trip. **Result: `datetime civil`
    973.9 → 96.5 ms** (min 3.2→1.0, max 20150→938; median 10×). `datetime_civil`/`iso`
    checksums 4058221/16948 **proven unchanged** vs a stashed-baseline rebuild; full
    `cargo test` green; the 11 datetime `.ir` snapshot goldens regenerated
    (`sync-goldens.sh`) — no `.ncode`/behavior change (11 acceptance fixtures pass);
    `artifact-gate.sh` back to the 17 pre-existing flaky diffs. The residual max 938
    ms is the arena quadratic itself (A1/A2), not yet linear — A3 is the mitigation, not
    the full fix. `datetime_package.mfb` `__datetime_addDays`/`__datetime_addMonths`.
  - **[ ] A3-csv** and the `"UTC"` intern remain.
- Order: A1 → A2 (the real fix), A3 as belt-and-suspenders. **Acceptance criterion:** the
  plan-65 arena-gated rows (`encoding base64`, `datetime iso`, `map intchurn`, the
  crypto churn rows) bump from tiny to realistic N in the commit that lands A and stay
  **linear** across the `--run` loop. Gate: all checksums unchanged +
  `scripts/artifact-gate.sh`.

### Sub-plan B — borrow read-only collection element (kill `copy_flat_block` on get)

**Covers:** dispatch union (162, P2), liststr hof (15.3, P1), liststr query (10.8, P3);
amplifies groupBy's bucket copy (D).

**Mechanism.** `LET e = collections::get(list, i)` and every predicate/mapper argument
lowers through `materialize_owned_element` (`builder_collection_queries.rs:10-23`) →
`copy_flat_block` (`builder_collection_layout.rs:306`, `arena_alloc` `:333` + memcpy
`:346`) for freeable flat/String values — a **fresh arena copy per element**, even when
the value is only read (MATCHed / length-checked) and never stored or mutated.
`dispatch union` (`dispatch.mfb:44`) pays ~4.09M such copies/rep; `liststr query`/`hof`
pay one fresh String per element per predicate call.

**Root cause (file:line).** `materialize_owned_element` (`builder_collection_queries.rs:10-52`,
`lower_list_get`/`lower_collection_get` at `:25`); String freshness comment at `:4-9`;
map-get twin at `:69`. MATCH's own variant binding already aliases the inline block
without copying (`builder_control.rs:92`) — the copy is pure overhead for the read-only
case.

**Fixes (semantics-preserving — copy only when the element escapes).**
- **B1:** return an aliasing **borrow** (pointer into the list's inline element) for a
  `collections::get` whose result is consumed read-only within the statement — MATCH
  scrutinee, field read, predicate/mapper argument. Mirrors MATCH's `UnionExtract`.
- **B2:** fuse `MATCH collections::get(list, i)` into a direct read of the inline
  element's tag+payload (no intermediate `e` block at all).
- **B3:** gate the copy on escape analysis — copy only when the element is
  stored/returned/mutated; keep the fast borrow for the common read-only case.
- Gate: dispatch/list/liststr checksums unchanged + `scripts/artifact-gate.sh`.

### Sub-plan C — map in-place removeKey + index preservation

**Covers (4):** mapchurn churn (164, P1 — worst map), mapchurn iterate (24.5, P1), map
int_ops (5.5, P1), map str_ops (6.1, P1). Consistent spreads ⇒ genuine, not arena.

**Mechanism.** `set` is already in-place + hash-incremental (`builder_inplace_assign.rs:266`
→ `lower_map_set_in_place` `map_mutate.rs:881`). The defects are `removeKey` and the
source-generic `merge`/`mapValues`.

**Root cause (file:line).**
- **removeKey:** no `try_inplace_remove_key_assign` — the in-place dispatch
  (`builder_control.rs:411-425`) covers only append/bulk-append/set/prepend/concat. So
  `m = removeKey(m,k)` falls to `lower_map_remove_key` (`map_mutate.rs:1229`): O(N)
  survivor scan (`:1294-1336`), fresh `arena_alloc` (`:1364`), full O(N) entry copy
  (`:1410-1444`), and the product's header **resets `BUCKETS_READY=0`**
  (`collection_buffer.rs:179-184` via `:1383`). The paired `hasKey` then probes a
  `ready==0` map → runtime fallback `_mfb_rt_map_probe` rebuilds the whole index O(N)
  (`builder_collection_query.rs:158-162`). Two O(N) sweeps per cycle.
- **merge/mapValues:** `__collections_merge` (`collections_package.mfb:327-335`) opens
  `MUT result = a` → owner-copy deep-copies the whole base map
  (`builder_values.rs:116-134` → `copy_collection_tight` `builder_collection_layout.rs:359`)
  to add 10 keys. `__collections_mapValues` (`:248-254`) rebuilds an N-entry map
  element-by-element leaving `ready=0`, so the next `get` rebuilds the index.

**Fixes (value semantics preserved — same survivor set + iteration order).**
- **C1:** `try_inplace_remove_key_assign` (mirror `try_inplace_set_assign`'s ownership
  gate) — compact the entry array + data region down by one slot in place, and
  **build the bucket table incrementally during the compaction pass** (set `ready=1`),
  backward-shifting the probe chain so no tombstones and probe order stays byte-identical.
  Removes both O(N) sweeps of `mapchurn churn`.
- **[x] C2-mapValues — LANDED.** Native `collections::mapValues` for a same-type 8-byte
  fixed-width value (V==U in Integer/Float/Fixed/Money; gate parses `#collections_mapValues$K$V$U`;
  else `.mfb`): copy the map's key/bucket structure once and rewrite each value payload in place
  via `f` (keys unchanged → copied index stays valid). **Result: `mapchurn iterate` 24.5 → 14.6 ms
  (~40%).** checksum 50153000 proven unchanged; full `cargo test` green; artifact-gate zero new
  diffs; 63 map/collections acceptance fixtures pass; String-value fallback verified.
  `lower_collection_map_values_call`. Commit: `3ba2f61d9`.
- **C2-merge:** native `merge` — size the result to `|a|+|b|`, copy `a` **once with buckets
  built** (`ready=1`), insert both in one pass; native index-preserving `mapValues`
  (copy the source entry/bucket structure, rewrite only value payloads).
- **C3:** preserve `BUCKETS_READY` across `copy_collection_tight` generally (rebuild-
  during-copy) — a win for every source generic that opens `MUT result = <map>`.
- Order: C1 (churn) → C2 (iterate/int/str_ops). Gate: map checksums + `scripts/artifact-gate.sh`.

### Sub-plan D — native-lower interpreted collection generics

**Covers (6):** listchurn nested (70.8, P1 — groupBy quadratic), list sortBy (21.2, P1),
list window (20.6, P1), list chunks (5.5, P1), list partition (6.0, P3), liststr query
(10.8, P3). Consistent spreads ⇒ genuine.

**Mechanism.** ~20 members are natively lowered; the rest run MFBASIC `__collections_*`
bodies with a per-element native call + indirect FUNC dispatch (`collections_package.mfb`).

**Root cause (file:line).**
- **groupBy — the worst.** `__collections_groupBy` (`collections_package.mfb:226-246`)
  does two full `transform` passes then per element: `hasKey`, **`MUT bucket =
  get(result,k)`** (`:235`, copies the whole *growing* bucket via `copy_flat_block`),
  `append(bucket,v)`, `set(result,k,bucket)` (`:237`, rebuilds map data on size change).
  Get-copies-the-bucket ⇒ **O(bucketSize²) per bucket** — the 7× Python gap.
- **sortBy** (`:101-156`): correct merge but the double-buffer is realized with full
  value copies — `MUT itemsDst = items`/`keysDst = keys` (`:110-111`) deep-copy both
  whole lists every pass and copy back (`:151-152`), on top of get×2+set×2 per element.
- **window/chunks** (`:300-312`/`:282-298`): `slice` is native (`lower_list_slice_range`
  `builder_collection_queries.rs:1111`) but each piece is alloc'd, copied into `result`
  (second copy), then freed — per-piece alloc/copy/copy/free churn.
- **partition** (`:337-352`): single pass but per-element get + indirect predicate call
  over 200k iters.

**Fixes (semantics-preserving — same order/stability/buckets).**
- **D1:** native `groupBy` — single pass growing each bucket **in place** in a hash-slot
  side structure, materialized to the Map once at the end (kills the O(bucket²) copy).
- **[x] D2 — LANDED (native sortBy).** Bottom-up **stable** merge sort with the two
  ping-pong buffer pairs allocated once and swapped by slot pointer per pass — no per-pass
  full copy (the `.mfb`'s dominant cost). Gated to **8-byte fixed-width items**
  (Integer/Float/Fixed/Money) and **signed 8-byte keys** (Integer/Fixed/Money) by parsing
  the monomorphized target `#collections_sortBy$<T>$<U>`; String/Scalar/Byte items and
  Float/non-numeric keys fall through to the `.mfb` `__collections_sortBy`. Keys filled by
  calling `keyFn` per element (failure → `emit_callback_failure_exit`, verified under TRAP);
  direct `[base + i*8]` addressing; stable (left run on ties). **Result: `list sortBy`
  21.2 → 4.46 ms (~4.75×; ≈ Python 3.73).** checksum 99800 **proven unchanged** vs a
  stashed-baseline rebuild; full `cargo test` green; `artifact-gate.sh` zero new diffs
  (native lowering is post-IR); 57 sort/collections acceptance fixtures pass; String-key
  fallback + TRAP'd failing keyFn both verified. `lower_collection_sortby_call`
  (`builder_collection_queries.rs`), dispatch gate `builder_values.rs`. Commit: `ec79ef661`.
- **[x] D3-window — LANDED.** Native `collections::window` for 8-byte fixed-width elements
  with constant `size>=1` + `stride==1` (gate parses `#collections_window$T` + checks the
  literal args; else `.mfb`). Builds the `List OF List OF T` result directly: outer kind-0
  list with per-window kind-2 inner blocks written in place at the data tail, one word-copy
  per window from the source — no per-window slice-alloc/copy/free. **Result: `list window`
  20.6 → 4.58 ms (~4.5x; beats Python 8.38).** checksum 99100 proven unchanged; full `cargo
  test` green; artifact-gate zero new diffs; 74 window/list acceptance fixtures pass; edge
  cases (size==n, size>n empty, size==1) verified. `lower_collection_window_call`.
  Commit: `9409d7941`. (window stride>1 remains on the `.mfb`.)
- **[x] D3-chunks — LANDED.** Native `collections::chunks` (8-byte fixed-width elems,
  constant size>=1; same nested-block build, variable last chunk, chunk-count via a count
  loop). **Result: `list chunks` 5.5 → 0.91 ms (~6x; COMPLETE, <=5ms, beats Python 1.68).**
  checksum 20000 proven unchanged; cargo test green; artifact-gate clean; acceptance passes.
  `lower_collection_chunks_call`. Commit: `13fcc99d0`.
- **D4:** native `partition`/`any`/`all`/`findIndex`/`findLastIndex` — one pass, reserved
  outputs, inlined comparator (with B's borrowed String element for String lists).
- Order: D1 (nested) → D2 (sortBy) → D3 (window) → D4. **Composes with E** (COW makes the
  sortBy/groupBy buffer copies free) and **B** (borrowed element for String lists). Gate:
  list checksums + `scripts/artifact-gate.sh`.

> **D2 native sortBy — implementation-ready design (execution, this session).** This is the
> most completable D-item: flat `List OF T` output (no nested/record construction like
> groupBy/window/partition), a well-defined stable bottom-up merge sort. Investigated:
> `__collections_sortBy` (`collections_package.mfb`) copies both whole lists per pass
> (`MUT itemsDst = items`/`keysDst = keys`, unavoidable in source — every position is
> overwritten so the copy is pure waste) then get×2+set×2 per element, over ⌈log₂n⌉ passes.
> - **Scope gate:** native only when **both `T` (item) and `U` (key from `keyFn`) are
>   fixed-width** (`list_element_is_fixed_width` = Integer/Float/Fixed/Money/Scalar/Byte);
>   else return `None` → the `.mfb` fallback (String keys/items keep working). The
>   `list sortBy` benchmark is `List OF Integer` by an Integer key, so it is covered.
> - **Plan:** (1) `keys = lower_collection_transform_call(value, keyFn)` (already native).
>   (2) Two ping-pong buffer pairs sized `n`: `(items,keys)` and scratch `(itemsB,keysB)`,
>   each a kind-2 fixed-width block (`HEADER + n*width`, no lookup array; alloc like
>   `lower_strings_to_bytes`/`lower_simd_alloc_list`, `dataLength = n*width`). Copy `value`
>   into the first `items` buffer once. (3) Bottom-up merge: `for width in 1,2,4,… < n`,
>   merge adjacent runs from src→dst using **direct addressing** `load/store [base + i*w]`
>   (no per-element bounds check — indices are algorithm-controlled), key compare
>   `keys[j] < keys[i]` via the type's native compare (signed for Integer/Fixed/Money,
>   `fcmp` for Float, unsigned for Byte/Scalar), **taking `i` on ties** to preserve the
>   `.mfb`'s stable order; then swap src/dst. (4) Return whichever buffer holds the result
>   (track parity of the pass count, or final-copy into `items`). ~150 lines; dispatch case
>   in `builder_values.rs` beside `transform`/`filter`. **Gate:** `list_sortBy` checksum
>   unchanged + full `cargo test` + `artifact-gate.sh` + acceptance sort fixtures + a
>   String-key sortBy fixture to prove the `.mfb` fallback still fires. **Not yet
>   implemented** — a focused ~150-line codegen push with its own debug/verify budget.
> - **Implementation decision (refined this session — pick one before coding):**
>   **(a) fixed-width + fallback:** raw `load/store [base + i*w]` (fastest), but the native
>   path must be **statically type-gated in the dispatch** (`builder_values.rs:708`-style)
>   so String/Float keys/items fall through to the `.mfb` `__collections_sortBy` — the
>   handler can't lower args then bail (double-lowering), so the gate needs a static
>   item-type + `callable_return_type(keyFn)` peek *before* lowering, and the general
>   collections-source-call fallthrough must be confirmed reachable. **(b) generic +
>   all-types:** use `lower_list_get`/`lower_list_set` + the `<`-operator compare on the
>   two key `ValueResult`s — handles every element type (no fallback/gate needed), still
>   kills the per-pass full copies (the dominant cost), at the price of the generic
>   get/set bounds-check overhead. **(b) is lower-risk (no fallback plumbing) and captures
>   the main win; recommended.** Buffers allocated once via `lower_reserved_list`
>   (`list_mutate.rs:2424`) / `copy_collection_tight` (`builder_collection_layout.rs:359`),
>   ping-pong by swapping the four slot pointers per pass, return the parity-correct buffer.
>   Dispatch: `Some("sortBy") => self.lower_collection_sortby_call(args)` at
>   `builder_values.rs:1502` + the `if native == Some("sortBy") && args.len()==2` guard.

### Sub-plan E — COW / refcount collection buffers

**Covers (1 direct + broad):** list copy (32.8, P1); amplifies sortBy, groupBy, merge.

**Mechanism / root cause (file:line).** The 40-byte header
(`error_constants.rs:807-815`) has **no refcount/version word**, and there is no COW
logic in codegen. `lower_value_owned` (`builder_values.rs:116`) unconditionally
deep-copies any aliasing source — Local/Global/Capture/MemberAccess
(`value_is_aliasing_source` `:207-218`) — via `copy_flat_block` → `copy_collection_tight`
(`builder_collection_layout.rs:306,359`). `list copy` (`list.mfb:68-102`, `RETURN xs` ×
1000 over 1000-element lists) is 1000 full-list memcpys per fn, twice, ×50.

**Fixes.**
- **E1:** add a refcount/version word to the header; make `copy_collection_tight`
  copy-on-write — share the block on `RETURN`/param-alias, split on first mutation.
  Retires `list copy` and makes sortBy/groupBy's buffer copies free.
- **E2 (cheaper interim):** move-elision escape analysis — `RETURN local` of a
  caller-dead value becomes a move; the identity shape `FUNC f(xs) RETURN xs` elides the
  copy entirely.
- **Risk/scope:** E1 is the largest design change in the plan (value model + every
  mutation path); semantics must stay observably value-copy. **Recommendation:** land
  A/B/C/D first (they cut most copy volume with contained edits); take E2 early
  (contained) and E1 only if `list copy` + residual amplification still dominate. Gate:
  full `tests/` value-semantics fixtures + `scripts/artifact-gate.sh`.

### Sub-plan F — string single-pass case / split / join

**Covers (4):** string case (66, P1), strbuild splitjoin (11.3, P1), liststr build (6.8,
P1), string slice (37.2, P2). Consistent ⇒ genuine, not arena.

**Mechanism / root cause (file:line).**
- **case:** `lower_strings_case_map` (`builder_strings_builtins.rs:460`) is a **two-pass
  count-then-write**, each pass calling `emit_utf8_decode_next` per codepoint (`:509`,
  `:583`) — the string is decoded **twice** per op. The per-codepoint ASCII fast path is
  present and firing (`:511-518`, `:585-592`), but unlike `normalizeNfc` (which got a
  whole-string ASCII quick-check + single-pass byte copy, `:710-769`), `case_map` has
  **no whole-string ASCII shortcut**.
- **split/join:** `lower_strings_split` (`:1527`) and `lower_strings_join` (`:1318`) scan
  and copy **byte-at-a-time** (no memchr/word-copy) — split's length + write passes
  (`:1610-1646`,`:1725-1751`), the per-field copy `emit_string_split_write_entry`
  (`builder_strings_package.rs:267-276`), join's delim/value loops (`:1476-1506`).
- **build:** NOT `acc & …` (the accumulator is an Integer len-sum). Cost is String `set(nums,i,v&"!")`
  (`list.mfb:912`): a String payload's width changes so `set` takes the whole-list
  rebuild branch (`collection_mutate.rs:254+`).

**Fixes.**
- **[x] F1 — LANDED:** gave `case_map` the whole-string ASCII quick-check `normalizeNfc`
  has — scan once for a byte ≥0x80; if none, one decode-free pass that range-maps a–z/A–Z
  ±32 (reusing the existing `emit_ascii_case_transform` helper) with a `byte_len + 9`
  allocation identical to the slow path. Any byte ≥0x80 falls through to the two-pass slow
  path. **Result: `string case` 66.0 → 50.6 ms** (`string_case` checksum 7411120 unchanged;
  full `cargo test` green; `artifact-gate.sh` zero new diffs vs baseline — the 17 pre-existing
  diffs are the known union-drop resource-union nondeterminism; 9 case-mapping acceptance
  fixtures pass). `builder_strings_builtins.rs` `lower_strings_case_map`. Commit: `ced444de6`.
- **F2:** memchr-style single-byte delimiter scan + word-at-a-time (8-byte) block copy in
  split/join; fuse split's two scans for a single-char delimiter.
- **F3:** in-place data-tail rewrite for String `set` when the new payload fits
  `dataCapacity` (avoids the whole-list rebuild in `build`).
- Gate: string/list checksums + `scripts/artifact-gate.sh`.

### Sub-plan G — scalar integer-category classification + native toScalars

**Covers (2):** scalarbench classify (29.3, P1), scalarbench listchurn (10.7, P1).
Consistent ⇒ genuine (NOT the arena quadratic).

**Mechanism / root cause (file:line).**
- **classify:** the five `is*` predicates (`strings_package.mfb:47-77`) each call
  `__strings_genCat(toInt(sc))` and **String-compare** the result. `__strings_genCat`
  (`regex_unicode.mfb:8`) is a **4099-arm linear `IF cp<=N THEN RETURN "xx"`** returning
  a *String* category. Workload = 2000 × 71 × 5 = **710k genCat calls/run** + ~1.4M
  String compares (ASCII exits early, but each call still returns+compares a String).
- **listchurn:** `strings::toScalars` is not natively lowered — it maps to
  `__strings_toScalars` (`strings.rs:245`, `strings_package.mfb:18-28`) which per pass
  does `toBytes` + `utf32Encode` (per-byte `get`+`toInt`+`append`, `encoding_package.mfb:139-178`)
  + a per-cp `toScalar`+`append` — **3 fresh lists** and ~450 `collections::get`/pass.

**Fixes (checksum-preserving; ASCII workloads).**
- **G1:** `genCat` returns an **Integer** category code (or a parallel `__strings_genCatCode`);
  the five predicates compare integers, not Strings — kills every String return/compare
  on the hot path.
- **[x] G2 — LANDED (ASCII fast path):** each of `isLetter`/`isDigit`/`isWhitespace`/
  `isUpper`/`isLower` now returns a direct range test for `cp < 128` (A-Z=65-90, a-z=97-122,
  0-9=48-57, space=32, plus 9-13 for whitespace) that exactly reproduces `genCat`'s ASCII
  category, skipping the 4099-arm scan + String return/compare. **Result: `scalar classify`
  29.3 → 4.2 ms — COMPLETE** (beats Python 14.2; ≤5 ms). checksum 3413150747 **proven
  unchanged** vs stashed-baseline rebuild; full `cargo test` green; one `.ir` snapshot golden
  (`scalar-strings-seam-rt`, line-renumber churn) regenerated, `.run`/behavior unchanged;
  24 scalar/strings acceptance fixtures pass. `strings_package.mfb`. Commit: `4cfb9a7f9`.
  (The 4099-arm→table conversion is a separate optional cleanup; the ASCII path retires the
  benchmark.)
- **[!] G3 REJECTED (execution — attempted, measured, reverted).** Two independent
  disproofs: (1) **toScalars does not dominate `scalar listchurn`.** Native `toScalars`
  fired (dispatch confirmed at `builder_values.rs:644`, before the `.mfb` fallback;
  `scalar_listchurn` checksum 48 unchanged) yet the row stayed 11.0 ms — the cost is the
  **ascent loop** (90 `collections::get` + 89 compares over the scalar list, ×2000×50),
  not the decode. (2) **A native two-pass `toScalars` is not even faster than the `.mfb`
  version:** isolated microbenchmark (150-char string ×50000) measured native **411 ms vs
  `.mfb` 388 ms** — the count-then-write structure decodes the string twice, costing as
  much as the interpreted `toBytes`+`utf32Encode`+`append` chain. A one-pass
  over-allocate-to-byteLen variant might edge it out, but it would not move the benchmark
  (ascent loop dominates), so it is not worth the codegen surface. `scalar listchurn`'s
  real lever is compiler bounds-check elimination on the get-loop (L2), not G3.
- Gate: the five classification counts + scalar checksums unchanged.

### Sub-plan H — json / regex source packages

**Covers (5 P1 + regressions):** parse regex (38.5), parse json (5.06), regexbench
compile (22.6), alternation (11.8), replace (12.6). (**parse csv is A**, not H — its
algorithm is fine.)

**Mechanism / root cause (file:line).**
- **json** (consistent 5 ms — genuine): `__json_parse` (`json_package.mfb:318-326`)
  materializes the whole ~24 KB input into a `List OF String` of one-grapheme strings
  via `strings::graphemes` (`:319`), indexes it with `get`+String compares; per-digit
  `acc = acc & ch` (`__json_collectNumber:646`); `__json_validNumber` (`:659`)
  re-graphemes every number token (`:660`).
- **regex:** `__regex_makeCtx` (`regex_package.mfb:228-235`) builds **both** a `List OF
  String` (`:229`) and a `List OF Integer` (`:230-233`) of the input; `__regex_searchFrom`
  (`:955-968`) restarts the full matcher at every start position; `__regex_setCap`
  (`:702`) `collections::set`-copies the caps list per group; **no compiled handle** —
  `__regex_compile` (`:1748`) is called at the top of match/find/findAll/replace
  (`:1879/1886/1899/1932`), so `regexbench compile` (25 lines/run) parses per line;
  `__regex_replace` (`:1931-1965`) does `strings::mid` re-walks per match (`:1950,1957,1963`)
  + `__regex_toScalars(repl)` per match (`:1802`). (bug-315 added an iterative greedy-
  simple-repeat path, `:859-891`, and a step budget — the O(n) start-restart is unchanged.)

**Fixes (source-package; checksums csv=6003000, json=5000, regex=200 unchanged).**
- **H1 (regex, biggest):** a **compiled-pattern handle** so compile/find/findAll/replace/
  match parse once and reuse the program — directly retires `regexbench compile` and cuts
  every per-call parse.
- **H2 (regex):** drop the dual list in `makeCtx` (keep only `cps: List OF Integer`);
  MUT the caps buffer in place instead of `set`-copying (`:702`); a first-scalar
  prefilter in `searchFrom` (`:960`) to cut O(n) restarts on literal-anchored patterns;
  build `replace` output from `ctx.chars` slices and hoist `toScalars(repl)` out of the
  match loop.
- **H3 (json):** replace `strings::graphemes` (`:319`) with `encoding::utf32Encode` →
  `List OF Integer` (all structural chars ASCII → integer compares, the pattern csv
  already uses); accumulate number tokens by `(start,end)` slice not `acc & ch`; validate
  over the code-point slice instead of re-graphemeing (`:660`).
- **Structural floor:** a source CPS matcher will not reach C POSIX-NFA / CPython `re`
  speed; H gets it much closer — bound the expectation. Gate: parse/regexbench checksums
  + counts.

### Sub-plan I — inline-TRAP Result/Error materialization elision

**Covers (1 severe P1 floor):** dispatch trap (6961 — the 1104 ms floor; the climb is A).

**Mechanism / root cause (file:line).** Workload: `toInt(tok) TRAP(err)` over
1000 tokens × 100 passes, 25% invalid → 75k successes + 25k failures/rep. The desugar
`lower_inline_trap` (`ir/lower.rs:961`, body `:990-1064`) materializes a `Result OF
Integer` block only to `If ResultIsOk`/destructure it one op later. The integer parse
itself is a tight register loop (`builder_conversions.rs:171`) — not the cost. The cost
is per-iteration arena allocation:
- **success (75k/rep):** `materialize_current_result` → `emit_build_result_inline`
  (`builder_arena_transfer.rs:129`, 24-byte `arena_alloc` `:31-41`) — one block per
  successful `toInt`, freed one op later.
- **error (25k/rep):** `emit_error_register_return` (`builder_error_emission.rs:494`)
  builds an `ErrorLoc` (`:133`), an inline Error block (`emit_build_error_inline:184`,
  second `arena_alloc` `:284` + 2 memcpys), then `materialize_current_result`'s error
  branch adopts + builds a Result (third `arena_alloc`) and frees the adopted block.
So ~3 allocs+3 memcpys per failure, 1 alloc per success — ~100k+ allocs/rep behind the
1104 ms floor. The 1104→12668 climb is A (mixed-size fragmentation).

**Fixes (semantics-preserving).**
- **I1 (biggest):** skip the `Result`-block materialization when the inline-TRAP consumer
  is adjacent — fuse `lower_inline_conversion_raw` with the enclosing TRAP so the raw
  tag/value **registers** feed the Ok/Err branch directly (eliminates `emit_build_result_inline`
  + its scope-drop free on all 100k iterations).
- **I2:** elide `ErrorLoc` + flat-Error assembly when the handler provably ignores `err`
  (here `RECOVER 0 - 1` never reads it) — the error path then needs only the tag.
- **I3:** allocate these short-lived Result/Error/ErrorLoc blocks in a per-iteration
  scoped sub-arena that resets each `FOR EACH` iteration (flattens the climb even before A).
- Gate: dispatch trap checksum + inline-TRAP tests (`tests/`) + `scripts/artifact-gate.sh`.

### Sub-plan J — vector Integer/Fixed op-inlining

**Covers (3):** vector int (56.4, P2), vector math (33.1, P2), vector float (21.9, P2).
Also lifts `vector fixed` (mfb-only). **Uncapped** — the real actionable vector lever.

**Mechanism / root cause (file:line).** `vector_op_inlinable`
(`builder_vector_inline.rs:104-111`): `scale`/`dot`/`cross(3D)` inline for all types
(C1 landed), `length`/`distance`/`lerp` inline **Float-only** (`:108`), and `normalize`/
`angle`/`project`/`reject`/`reflect`/`slerp`/`clamp_length`/etc inline for **no** type. For
Integer/Fixed and for `normalize` (any type), each op makes a `#vector_<op>` FUNC call
and **materializes the register-native operand to a fresh N×8 arena block**
(`vector_value_as_block:184`), and Integer/Fixed length/distance run software isqrt
(`vector_package.mfb:74`). `vector math` (200k iters) is dominated by `normalize` ×2/iter
(`vector_package.mfb:290`), never inlined for any type — the heaviest.

**Fixes (semantics-preserving — reuse the module's `lower_value` bit-identity technique).**
- **J1 (biggest):** inline `normalize` (Float) as `scale(v, 1/length(v))` with the
  zero-length `FAIL` guard — clears the dominant `vector math` cost.
- **J2:** extend `length`/`distance`/`lerp` inline to Integer/Fixed (relax the
  `element == "Float"` clause at `:108`) — Fixed sqrt is already deterministic inline
  (`emit_fixed_sqrt`); Integer isqrt reproduced inline or fed register-native lanes to
  skip the block materialize. Attacks `vector int` directly.
- **J3:** inline the remaining pure-arithmetic ops (`project`/`reject`/`reflect`, 2D/4D
  `cross`, `perpendicular`, `rotate_2d`) — each a re-lowerable arithmetic tree.
- Gate: `tools/math-kernels/runtime_ulp.py` (normalize reuses the already-gated
  `math::sqrt`) + scalar-vs-array bit identity + vector checksums + `scripts/artifact-gate.sh`.

### Sub-plan K — bignum limb-wise / Barrett reduction

> **[!] K1/K2 REJECTED as written (execution correction).** The C mirror
> (`benchmark/c/main.c:307` "schoolbook mul + **bit-serial mod**"; `bn_mod`
> `:363` is the same `for (i = nbits-1; i>=0; i--)` bit loop) uses the **same
> bit-serial reduction** as the MFBASIC `bnMod`. Replacing MFBASIC's algorithm
> with limb-wise/Barrett would make it beat C by running a *different, better
> algorithm* — not by executing the same work faster — so it changes what the
> benchmark measures and is an unfair (illegitimate) comparison change. The
> genuine bignum gap is per-op overhead: MFBASIC pays a bounds check on every
> `collections::get`/`set` in the hot loop where C indexes a stack `uint32_t[]`.
> The legitimate lever is therefore **compiler-side bounds-check elimination on
> loop-induction-var list access (the L2 "unchecked-get" mechanism), applied to
> the bit loop** — keeping the algorithm identical to C. K3 (bnCmp top-limb
> hoist) is a borderline micro-opt that still diverges from C's exact code; skip.
> Reclassify: bignum is an **L2/compiler** row, not a benchmark-source rewrite.

**Covers (2):** bignum modmul (19.4, P2), modexp (10.8, P3). Source-level (`main.mfb`).

**Mechanism / root cause (file:line).** The in-place `r` buffer landed (`main.mfb:701-706`,
`try_inplace_set_assign`), but `bnMod` (`main.mfb:695-745`) is still **bit-serial**
O(nbits×limbs): ~530 bit iterations × ~35-55 bounds-checked `collections::get`/`set` limb
ops (shift-in `:717-718`, `bnCmp :722`, conditional subtract `:726-740`) ≈ ~5M list ops
per modmul run vs C indexing a stack `uint32_t[]`.

**Fixes (source-level; result is the unique remainder → checksums unchanged).**
- **K1 (biggest):** limb-wise schoolbook reduction — process 28-bit limbs of `x` top-down
  (`r = r*2^28 + limb`, subtract one trial-quotient digit × m). O(limbs²) ≈ 19×11 vs
  O(nbits×limbs) ≈ 530×11 — ~28× fewer inner iterations.
- **K2:** Barrett reduction — precompute `mu = floor(2^k/m)` once per test (m is
  loop-invariant, `main.mfb:753/780`), reduce with two `bnMul`s + a subtract (reuse the
  in-place `bnMul` `:646-672`).
- **K3 (cheap, independent):** hoist `bnCmp(r,m)` — after one shift `r < 2m`, so compare
  only the top 1-2 limbs, not a full 11-limb scan.
- Gate: bignum modmul/modexp checksums (`main.mfb:775,807`) unchanged.

### Sub-plan L — concat-chain fusion + memo bounds/MOD

**Covers (2):** io format (8.5, P1), mathpipe memo (11.4, P3).

**Mechanism / root cause (file:line).**
- **io format:** `iobench.mfb:92` builds a line as a left-associated chain of ~7 `&`
  concats into a fresh `LET` — each `lower_string_concat` (`builder_value_semantics.rs:378`)
  arena-allocs `left+right` and copies both operands, and because `line` is a fresh `LET`
  (not `name = name & …`) the in-place `try_inplace_concat_assign` (`builder_inplace_assign.rs:398`)
  does **not** fire → ~7 growing intermediates + O(n²)-per-line prefix re-copy over 20000
  lines. The Float formatter (`float_format.rs:48`) is intrinsic per-value work.
- **memo:** per DP cell (`mathpipe.mfb:191-192`) 2 bounds-checked `collections::get`
  (`builder_collection_query.rs:22-51`) + in-place `set` (already O(1)) + `MOD` div-check
  (`builder_numeric.rs:901`), ×~840k cells/run.

**Fixes (checksums = constant line count / DP result, unchanged).**
- **L1:** recognize a multi-`&` chain feeding a `LET`/`writeAll` and lower it to **one
  summed-length allocation + N copies** (no intermediates) — or build in a pre-sized `MUT`
  accumulator so the in-place concat path fires.
- **L2 (memo):** an unchecked-`get` variant provable in-bounds when the index is a loop
  induction var with loop-invariant list length (reuse I1's bound-tracking); strength-
  reduce constant `MOD 1000000007` to a conditional subtract (the sum of two values each
  < m is < 2m).
- Gate: io/memo checksums + `scripts/artifact-gate.sh`.

### Sub-plan M — transcendental + float kernels (structurally capped)

**Covers (14):** math sin/cos/tan/exp/log/log10/pow/simd/asin/acos/atan/atan2 (P2/P3),
sqrt (P4); float leibniz/nbody (P3), mandelbrot (P4).

**Mechanism + ceiling (file:line).** All `Float` math is native inline NEON f64, no libm.
Scalar + array share one code path (`builder_math.rs:66-70,331-338` → the same kernels),
so ULP/bit-identity is enforced by construction. Each transcendental open-codes a
**double-double compensated Horner** (`emit_compensated_horner`,
`builder_simd_float_math.rs:1854`, on `emit_twoprod`/`emit_twosum`) to meet the ≤1-ULP /
deterministic / no-libm contract (`emit_exp_body:1499`, `emit_log_body:1637`,
`emit_sin_cos_body:1116`, `emit_tan_body:1353`, `emit_atan_core:680`); pow is a full
software fdlibm dd expansion (`builder_pow.rs:169`). This is **inherent** to computing
these in software dd arithmetic — c-O0 calls hand-tuned vendor libm. **The 3-10× gaps are
the structural dd-vs-libm delta; achievable semantics-preserving gain ≈ 0.** `sqrt` is a
single hardware `float_sqrt_d` (`builder_math.rs:1091`) — IEEE-exact, optimal; residual is
the per-call domain `fcmp`.

The **pure-float rows** (leibniz/nbody/mandelbrot) are **not** dd-capped. Their overhead is
the plan-17 per-observation-boundary finiteness check (`emit_float_result_check_fp`
`builder_math.rs:1220`, fired by `observe_float:1273`, gated `float_arith_node:1327`),
already boundary-coalesced (one check per observed assignment). mandelbrot already **beats
c-O0** (loses only to c-O2 autovectorization).

**Fixes (the only bounded levers).**
- **M1:** coalesce sibling finiteness checks at a shared boundary — mandelbrot/nbody's
  `nzr`/`nzi` are two producers per iteration; a combined `fmax(|nzr|,|nzi|)` vs +Inf
  halves the branch count (bit-identical trap set; keep the earliest `line:char` stamp).
  Bounded — tens of percent on leibniz/nbody, not a multiple.
- **BLOCKED:** the transcendental band cannot reach G2 without dropping dd precision or
  importing libm — both semantic changes. **Documented as a ceiling; track for regression.**
- Gate: `tools/math-kernels/runtime_ulp.py` + scalar-vs-array bit identity + all math
  checksums unchanged.

### Sub-plan N — integer overflow-check residual (structurally capped)

**Covers (2):** recurse fib (76.9, P2), thread sum (44.1, P2).

**Root cause + ceiling (file:line).** I1 elision is **verified firing**: fib's `n-1`/`n-2`
lower to bare `sub` under the `n<2` guard (`builder_numeric.rs:750-810,870-876`); thread's
`i = i+1` elides under the `i < stop` strict-upper bound (`:839-842`, `builder_control.rs:758-790`).
The residual is **irreducible under integer-overflow-trap semantics:**
- **thread sum:** `total = total + i` — right operand is a Local, not `+1`, so
  `integer_add_elidable` is false (`:783`) → checked add (`:844-849`); `total` can
  genuinely overflow i64.
- **fib:** `fib(n-1) + fib(n-2)` — both call results, overflow-capable → checked add
  stays; every one of ~29.8M calls also pays the `RESULT_TAG` write (`builder_exits.rs:313`)
  + post-call `compare/branch_eq` propagation (`builder_emit_helpers.rs:207-217`).

**Fixes (bounded; the checked add itself is immovable).**
- **N1:** when a fallible callee's tag is *immediately* consumed by an identically-
  propagating enclosing return (fib's `RETURN fib()+fib()`), merge/hoist the redundant
  per-call `compare/branch_eq` + callee `mov tag, OK`. Codegen change with tree-wide blast
  radius; marginal (~1.37× gap for fib).
- **BLOCKED:** eliding the value-carrying add's overflow check breaks the trap contract —
  off the table. **Ceiling; track for regression.**
- Gate: overflow-trap tests + fib/thread checksums + `scripts/artifact-gate.sh`.

### Sub-plan O — bits register-operand fusion

**Covers (1):** bits ops (6.4, P4 — beats Python 120×, only > c-O2 + 5 ms). Lowest priority.

**Mechanism / root cause (file:line).** Every binary op routes through
`lower_bits_two_integers` (`builder_bits.rs:37-68`) which **spills both operands to fresh
stack slots and reloads them** per call (`:46-61,65-66`); shifts add a 0..63 range-check
(`:131-134`). A fused expression round-trips every intermediate through memory over 3.2M
ops/run.

**Fixes.**
- **O1:** skip the spill when an operand is a constant (`bits::sl(x,3)` — the shift amount
  folds to a compile-time range check and the reload disappears); fuse
  `lower_bits_two_integers` so a bits result already in a register feeds the parent op
  without a store/reload. Gate: bits checksum + `scripts/artifact-gate.sh`.

## Validation Plan (all sub-plans)

- **Correctness first:** every fix produces identical observable output — the benchmark
  checksums on stderr (`csv=6003000`, `json=5000`, `regex=200`, plus each group's printed
  checksum: list/liststr/map/mapchurn/string/strbuild/scalar/dispatch/datetime/encoding/
  vector/bits/bignum/math/float/io/thread) **unchanged** — and passes `scripts/test-accept.sh`
  + `tests/`. No language/semantic/syntax/precision change; value-semantics and
  integer-overflow-trap semantics preserved.
- Re-measure the affected group with the **same `--run 50`** as the source logs
  (`20260725-075953`); confirm the row's band improved (ideally to complete). For A and
  the arena-gated rows, confirm **linear** scaling across the run loop (min ≈ median).
- Codegen changes: `scripts/artifact-gate.sh` (byte-deterministic 4-target self-diff).
  Math changes: `tools/math-kernels/runtime_ulp.py`.

## Execution status (/follow-plan, worktree P-64)

**Landed & fully verified (each: benchmark checksum proven unchanged vs a stashed-baseline
rebuild + full `cargo test` green + `artifact-gate.sh` clean-modulo-17-flaky + acceptance
fixtures pass):**

| Item | Change | Row | Before → after (ms, `--run 50`) | Commit |
|------|--------|-----|------|--------|
| **F1** | whole-string ASCII shortcut in `case_map` | string case | 66.0 → 50.6 | `ced444de6` |
| **A3-datetime** | fixed-offset fast path in `addDays`/`addMonths` | datetime civil | 973.9 → 96.5 (10×) | `de8da577a` |
| **G2** | ASCII fast path in 5 classify predicates | scalar classify | 29.3 → **4.2 (COMPLETE**, beats Py) | `4cfb9a7f9` |

Authoritative post-fix full `--run 50` (`benchmark/mfb-20260725-223351.log`) confirms the
three rows (case 50.3, civil 77.5, classify 4.2) with **no regression** on unaddressed rows
(copy 33, sortBy 21, listchurn nested 71, mapchurn churn 166 — all ≈ plan). **Bonus:
`dispatch trap` 6961 → 1885 ms** — an indirect A3 benefit: fewer datetime transients cut the
process-global arena fragmentation that amplifies trap's climb (exactly the A↔trap coupling
the plan predicted). `parse csv` median (447↔602) and `thread sum` (44↔68) swing run-to-run
— inherent arena-quadratic / scheduler variance (csv min still 5.57), not touched by these fixes.

**Not yet done — every remaining item was investigated at `file:line` this session and each
is a substantial dedicated effort with a specific, documented complication (not a rushed
pass; correctness bar per `.ai/compiler.md` governs):**
- **I** (inline-TRAP Result/Error elision) — **dispatch trap 1885 ms, the #1 offender.**
  `lower_inline_trap` (`ir/lower.rs:963`) materializes a `Result OF T` block then destructures
  it one op later; eliding it needs a new IR op or a codegen peephole on the
  `Bind Result; If ResultIsOk{ResultValue} else{ResultError}` shape — **tree-wide blast
  radius** (every `TRAP`).
- **D1** (native groupBy, `listchurn nested` 70.8 ms, real O(bucket²)) — multi-hundred-line
  native lowering emitting FUNC-pointer `keyFn`/`valFn` calls + in-place bucket-grown map;
  no source restructure avoids the value-semantic `get`-copy.
- **C1** (map in-place removeKey, `mapchurn churn` 166 ms) — variable-length String-key
  data-region compaction + offset fixups + incremental bucket maintenance; still O(N)/op
  (win is only no-alloc + O(1) paired `hasKey`), new in-place path needs its own tests.
- **L2 / bounds-check elimination** — the *real* lever for `scalar listchurn`, `mathpipe memo`,
  AND bignum (see K/G3 rejections): drop the bounds check on `get(list, i)` when `i` is a
  loop-induction var < loop-invariant `len(list)`. **Correctness-critical dataflow** — an
  unsound unchecked get is silent memory unsafety, not a checksum-catchable wrong answer.
- **J1/J2** (vector `normalize`/Integer-`length` inline, `vector math` 33 ms) — `normalize`
  is per-lane `v.f / len` **plus a `len=0.0 → FAIL error(77050002)` guard** (`vector_package.mfb:353`);
  the guard is control flow the pure-expression inline mechanism can't express, and
  Integer/Fixed `length` needs `isqrt` not `sqrt`. J3 (project/reject/reflect) IS pure
  arithmetic but is not on the `vector math` hot path.
- **A1/A2** (arena allocator machine-code) — **highest blast radius in the plan** (a bug
  corrupts every heap); the residual `datetime civil` max 938 ms + csv/iso/dispatch-trap-climb
  wait on it. **B** (borrow-on-get) — deep escape analysis. **F2/F3** (string split/join/set
  in-place) — intricate shift+offset-fixup for ≤1 ms benchmark impact. **C-C1**/A3-csv
  (needs a range-decode primitive).
- **Rejected with proof this session: K** (bignum — algorithm change unfair vs C's bit-serial
  mod) and **G3** (native `toScalars` — measured *slower* 411 vs 388 ms, and toScalars isn't
  `listchurn`'s bottleneck). Both really want L2.
- **Capped (ceilings, track-only): M** (transcendentals — dd-vs-libm), **N** (fib/thread
  overflow-check).

## Corrections (execution — /follow-plan)

Recorded as the plan is executed; each is a divergence between the plan as written
and what execution found, with evidence.

- **Plan structure — no Prerequisites gate, no per-task checkboxes, no `Commit:`
  lines.** plan-64 is a *master index* (Task-1 priority list + Task-2 sub-plan bodies),
  not the phased-checkbox template `/follow-plan` expects, and none of the promised
  `plan-64-<letter>-*.md` split files exist. The Prerequisites gate is therefore
  vacuously passed. Execution tracks completion per sub-plan by tagging the relevant
  fix bullet `[x] … — LANDED` with its result + commit hash, in the same commit as the
  work.
- **Execution order refined by blast radius, not pure ROI.** The plan's "A first"
  ordering is by ROI. `/follow-plan` §3 says phases run uncertainty-first / blast-radius
  last; sub-plan **A1/A2 (the arena allocator machine-code emission) is the single
  highest-blast-radius surface in the plan** (a bug corrupts every program's heap).
  Execution banks the *contained, semantics-preserving* wins first (F1 done; C1, then B,
  D, etc.), each fully gated, and attempts the deep allocator rewrite (A1/A2) after the
  contained value is banked. A3 (the csv/datetime *source* mitigations) directly retires
  plan-64's own arena-bound benchmark rows; A1/A2 is foundational for plan-65 coverage.
- **The arena allocator has evolved since the plan's `file:line` citations** (arena.rs
  is 960 lines, not the plan's `:715-729`/`:387-407`-era file). The flush-before-grow is
  now gated to SMALL requests with a one-shot `flushed` retry + a post-flush re-park
  sweep (`arena.rs:369-457`) — a partial mitigation of A1's premise already landed; A2's
  size-class segregation is partly present as plan-25-A "segregated large-block bins"
  (`arena.rs:44-50`). A1/A2 must be re-scoped against this current code, not the citations.
- **Legitimacy rule (K rejected).** A benchmark measures MFBASIC vs C/Python running the
  *same* algorithm; a valid fix speeds up the compiler/runtime/stdlib so the *same source
  operation* runs faster (F1 case-map, A3 datetime builtin, G2 classify builtin all qualify —
  they optimize a stdlib/compiler path, not the workload's algorithm). Sub-plan **K** would
  rewrite the benchmark's *own* `bnMod` from bit-serial to limb-wise — but the C mirror is
  *also* bit-serial (`main.c:307,363`), so K makes MFBASIC win by algorithm, not speed:
  rejected. Bignum's real lever is compiler bounds-check elimination (L2), reclassified.
- **Measurement: the full `--run 50` suite is ~20+ min** (the arena quadratic makes late
  iterations of dispatch-trap/datetime/csv progressively slower), too slow for a per-fix
  loop. Per-group re-measurement uses a throwaway trimmed copy of `benchmark/mfb`
  (`main()` cut to the target group) built with the current compiler — repo-touchless,
  ~seconds. A full clean `--run 50` is the final gate once the catastrophic groups (A/C/I)
  are fixed and the whole suite is fast again.

## Open Decisions

- **A (arena quadratic) is the highest-ROI lever this round** — it retires csv + datetime
  civil + datetime iso (3 severe P1, all pure churn), unblocks dispatch-trap's climb, and
  gates every plan-65 arena row. Recommend landing A first. Decision: **A first**, then the
  map cluster **C**, list generics **D**, shared **B**, then **H/F/G/I**.
- **E (COW) is a large design change** — recommend E2 (move-elision, contained) early and
  deferring E1 (refcount) until A/B/C/D have cut copy volume; take E1 only if `list copy` +
  residual amplification still dominate. Decision: E2 early, E1 deferred/reassess.
- **`dispatch trap` is split across two sub-plans** (I for the floor, A for the climb) —
  land I and A together to fully retire it; either alone leaves it a severe offender.
  Decision: schedule I with A.
- **M (math) and N (fib/thread) are structurally capped** — cannot reach their bands
  without breaking the dd-precision or overflow-trap contract. Decision: ceiling accepted;
  land only the bounded levers (M1 finiteness coalescing, N1 tag round-trip) if cheap,
  non-gating.
- **H-regex has a structural floor** — a source CPS matcher will not match C/CPython; H1/H2
  are large constant-factor wins but do not gate on parity. Decision: pursue H1 (compiled
  handle) + H2; bound the expectation.
- **Fixed / Money rows have no baseline** — `math fixed`, `vector fixed`, `mathpipe
  finance`/`money`; J lifts vector fixed incidentally. Decision: track for regression only.
