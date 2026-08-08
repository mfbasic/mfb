# plan-86: Benchmark performance — close the gap to C/Python

Last updated: 2026-08-04
Effort: xlarge (multi-day; many independently-landable `plan-86-<letter>` sub-plans)
Platform under test: **aarch64 / macOS** (the target these logs were taken on)

Source logs (one matched timestamp `20260804-083935`, **`--run 10`**):
`benchmark/mfb-<ts>.log`, `benchmark/c-O0-<ts>.log`, `benchmark/c-O2-<ts>.log`,
`benchmark/python-<ts>.log`. Startup is excluded (every workload is timed internally
with `datetime::monotonicNanos()`); **median** is the metric. Re-measure any fix with
the **same `--run 10`** as these logs.

This is the master plan + Task-1 ordered priority list for the benchmark performance
push. It scores every row in the current logs against the goals, orders the work, and
indexes the fix sub-plans (Task 2). The coverage plan is a separate document,
`planning/plan-87-benchmark-coverage.md`.

This is a **full fresh round** — every root cause below was re-derived from the current
tree at `file:line` this session (six parallel research passes), not carried forward.
**The predecessor plan-64 largely landed** (A1/A2 arena, D1–D4 native list generics,
C2-mapValues, F1 case ASCII, G2 classify, I2 trap-elision — all confirmed by the improved
rows below), and its completed sub-plans are archived in `planning/completed/`. Since
plan-64 the suite was **restructured** (commits `6261011c9`…`a601fd4b1`): the bug-430
**collection-container matrix** replaced the single `list`/`liststr`/`map`/`set` groups
with a `Fixed`/`Dynamic` × plain/`Record`/`State` grid, and C/Python peers were added for
the plain `Fixed`/`Dynamic` (and map `key-*`) groups. That exposed a large class of
**String-element (`Dynamic`) offenders** that were always slow but never had a scored peer
before — most of this round's P1s. Row counts are therefore not comparable to plan-64's;
see Movement.

## The goals (priority order)

A benchmark's **priority = the first goal it fails**. Work lowest-numbered failures first.

1. **G1** — mfb (MED) **< python** (MED).
2. **G2** — mfb ≤ c-O0 + **10 ms**.
3. **G3** — mfb ≤ c-O0 + **5 ms**.
4. **G4** — mfb ≤ c-O2 + **5 ms**.

**Override:** any mfb MED **≤ 5 ms is already complete**, regardless of G1–G4 (measurement
noise). A benchmark is otherwise complete only when it beats all four. Rows with **no
cross-language baseline** (the `Record-*`/`State-*` matrix + `math fixed`/`vector fixed`/
`mathpipe finance`/`money`) are excluded from G1–G4 scoring; regression-track only.

## Scorecard summary

| Bucket | Count | Meaning |
|--------|------:|---------|
| **P1** (fails G1, loses to Python) | 37 | highest priority |
| **P2** (fails G2, > c-O0 + 10 ms) | 18 | |
| **P3** (fails G3, > c-O0 + 5 ms) | 6 | |
| **P4** (fails G4, > c-O2 + 5 ms) | 3 | lowest priority |
| no-baseline (mfb-only) | 235 | excluded from scoring |
| complete (passes all 4, or ≤ 5 ms) | 178 | done |

Total = **477** rows (64 scored offenders across P1–P4). The suite ~3.7×'d since plan-64
(129 rows) via the bug-430 matrix (list/map/set × 6 containers, per-op split) + the plan-65
crypto/serialize groups + the per-op `bits` split.

**Movement since plan-64** (`20260725-075953`; note the restructure — most deltas are
newly-peered rows, not regressions):

- **plan-64 fixes confirmed landed** (rows now complete or greatly improved): `datetime civil`
  973.9 → **0.775** (A), `scalar classify` 29.3 → **4.18** (G2), `dispatch trap` 6961 →
  **3.46** (I2+A), `parse csv` 447 → **5.01** (A3), `string case` 66 → **48.7** (F1),
  `listchurn nested` 70.8 → **12.3** (D1), `mapchurn iterate` 24.5 → **14.27** (C2). The
  **`list (Fixed)` (Integer) matrix is almost entirely complete** (`sortBy` 4.40, `window`
  4.66, `chunks` 0.93, `groupby` 0.19, `partition` 4.75, `reduce` 1.69) — the D1–D4 native
  lowerings landed.
- **New P1 class — the `list (Dynamic)` (String) matrix.** The exact ops that are complete
  for Integer are P1 offenders for String: `reduce` 3048, `reduceRight` 877, `groupby` 166,
  `window` 88, `sortBy` 61, `sort_*` ~23–29, `zip` 25, `flatten` 24, `partition` 18.7,
  `chunks` 13.6. Root: plan-64's native lowerings are **gated to 8-byte fixed-width** and
  String falls through to the interpreted `.mfb` bodies (sub-plan **A**). Not a regression —
  these paths were always interpreted; the matrix + peers made them *scored*.
- **New P1 class — the set-algebra matrix.** `set (Fixed)` `union` 110, `toSet`/`symmetricDifference`
  ~70, `intersection`/`difference` 17 newly have C/Python peers → newly scored (sub-plan **C**).
- **New P1 — `crypto sha256`** (plan-65) 9.01: software `bits` core vs Python hashlib's C
  backend — structural, sub-plan **L**.
- **No algorithmic regressions found.** `mapchurn churn` (165) and `dispatch union` (160)
  are unchanged carry-overs (sub-plans D, E). The math/float/fib/thread band is unchanged and
  structurally capped (L).

---

## Task 1 — ordered priority list

Within each band, worst-first by mfb median. `Δpy`/`ΔO0`/`ΔO2` are `mfb − baseline` (ms).
**Sub-plan** maps each row to its fix (Task 2).

### P1 — loses to Python (fails G1) — do these first

| # | group/bench | mfb | py | Δpy | Sub-plan |
|--:|-------------|----:|---:|----:|----------|
| 1 | list (Dynamic) **reduce** | 3048.3 | 49.3 | +2999.0 | **B** reduce leak (+ O(n²) concat structural) |
| 2 | list (Dynamic) **reduceRight** | 877.7 | 51.8 | +825.9 | **B** native reduceRight |
| 3 | list (Dynamic) **groupby** | 166.2 | 0.13 | +166.1 | **A** String native lowering |
| 4 | mapchurn **churn** | 165.3 | 1.20 | +164.1 | **D** in-place removeKey |
| 5 | set (Fixed) **union** | 110.7 | 0.07 | +110.7 | **C** native set-algebra |
| 6 | list (Dynamic) **window** | 88.2 | 8.68 | +79.5 | **A** |
| 7 | set (Fixed) **symmetricDifference** | 70.4 | 0.08 | +70.3 | **C** |
| 8 | set (Fixed) **toSet** | 69.2 | 0.06 | +69.2 | **C** |
| 9 | list (Dynamic) **sortBy** | 61.3 | 3.22 | +58.1 | **A** |
| 10 | string **case** | 48.7 | 27.90 | +20.8 | **F** single-pass/memchr |
| 11 | list (Dynamic) **sort_rand** | 28.9 | 3.15 | +25.7 | **A** native sort |
| 12 | list (Dynamic) **zip** | 25.6 | 1.77 | +23.8 | **A** |
| 13 | regexbench **replace** | 25.1 | 0.03 | +25.1 | **I** regex (capped floor) |
| 14 | list (Dynamic) **flatten** | 24.6 | 2.58 | +22.0 | **A** |
| 15 | list (Dynamic) **transform** | 23.6 | 14.64 | +9.0 | **K** String-alloc/COW |
| 16 | list (Dynamic) **sort_asc** | 22.9 | 1.46 | +21.4 | **A** |
| 17 | list (Dynamic) **sort_desc** | 22.6 | 1.46 | +21.1 | **A** |
| 18 | regexbench **alternation** | 19.1 | 0.01 | +19.0 | **I** (capped floor) |
| 19 | list (Dynamic) **partition** | 18.7 | 8.31 | +10.4 | **A** |
| 20 | set (Fixed) **intersection** | 17.1 | 0.06 | +17.0 | **C** |
| 21 | set (Fixed) **difference** | 17.0 | 0.06 | +17.0 | **C** |
| 22 | list (Dynamic) **set** | 14.3 | 0.15 | +14.1 | **K** out-of-line String layout (bug-430) |
| 23 | mapchurn **iterate** | 14.27 | 7.55 | +6.7 | **D** native merge |
| 24 | list (Dynamic) **chunks** | 13.6 | 1.69 | +12.0 | **A** |
| 25 | list (Dynamic) **copy** | 12.5 | 1.14 | +11.4 | **K** COW |
| 26 | listchurn **nested** | 12.3 | 10.06 | +2.3 | **A** native flatten |
| 27 | strbuild **splitjoin** | 11.35 | 6.40 | +4.9 | **F** memchr split/join |
| 28 | scalarbench **listchurn** | 10.64 | 9.32 | +1.3 | **G** bounds-check elim |
| 29 | crypto **sha256** | 9.01 | 0.05 | +9.0 | **L** software-vs-C (capped) |
| 30 | list (Dynamic) **insert** | 8.86 | 0.30 | +8.6 | **K** (reflow inherent) |
| 31 | list (Dynamic) **removeAt** | 8.78 | 0.10 | +8.7 | **K** (reflow inherent) |
| 32 | io **format** | 8.37 | 6.85 | +1.5 | **L** formatter-capped |
| 33 | set (Dynamic) **union** | 7.43 | 0.05 | +7.4 | **C** |
| 34 | map **str_ops** | 5.97 | 2.82 | +3.1 | **D** String mapValues + removeKey |
| 35 | list (Fixed) **sort_rand** | 5.07 | 1.85 | +3.2 | **A** native sort |
| 36 | set (Dynamic) **add** | 5.07 | 0.05 | +5.0 | **C** in-place add |
| 37 | parse **csv** | 5.01 | 0.73 | +4.3 | **J** native csv parse |

### P2 — > c-O0 + 10 ms (fails G2)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | dispatch **union** | 160.6 | 14.96 | +145.6 | **E** borrow read-only element |
| 2 | math **pow** | 88.96 | 18.05 | +70.9 | **L** (capped) |
| 3 | recurse **fib** | 76.04 | 56.07 | +20.0 | **L** (capped) |
| 4 | math **tan** | 71.72 | 9.27 | +62.4 | **L** (capped) |
| 5 | vector **int** | 55.71 | 6.57 | +49.1 | **H** vector inline |
| 6 | math **simd** | 50.12 | 9.64 | +40.5 | **L** (capped) |
| 7 | thread **sum** | 40.63 | 9.26 | +31.4 | **L** (capped) |
| 8 | string **slice** | 36.68 | 22.90 | +13.8 | **F** |
| 9 | math **log10** | 36.38 | 7.79 | +28.6 | **L** (capped) |
| 10 | math **log** | 34.67 | 7.71 | +27.0 | **L** (capped) |
| 11 | math **cos** | 31.95 | 7.88 | +24.1 | **L** (capped) |
| 12 | math **sin** | 31.63 | 7.88 | +23.7 | **L** (capped) |
| 13 | vector **math** | 30.95 | 4.53 | +26.4 | **H** (normalize) |
| 14 | math **acos** | 21.81 | 8.62 | +13.2 | **L** (capped) |
| 15 | math **asin** | 20.95 | 9.91 | +11.0 | **L** (capped) |
| 16 | vector **float** | 20.90 | 6.30 | +14.6 | **H** |
| 17 | math **exp** | 20.36 | 7.75 | +12.6 | **L** (capped) |
| 18 | bignum **modmul** | 19.50 | 5.07 | +14.4 | **G** bounds-check elim |

### P3 — > c-O0 + 5 ms (fails G3)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | math **atan2** | 21.95 | 13.82 | +8.1 | **L** (capped) |
| 2 | float **nbody** | 19.03 | 11.90 | +7.1 | **L** (M1 finiteness lever) |
| 3 | math **atan** | 17.03 | 7.90 | +9.1 | **L** (capped) |
| 4 | mathpipe **memo** | 11.53 | 1.84 | +9.7 | **G** bounds-check elim + MOD |
| 5 | list (Dynamic) **findLastIndex** | 11.11 | 1.19 | +9.9 | **A** native lowering |
| 6 | bignum **modexp** | 10.86 | 2.78 | +8.1 | **G** bounds-check elim |

### P4 — > c-O2 + 5 ms (fails G4)

| # | group/bench | mfb | c-O2 | ΔO2 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | float **mandelbrot** | 51.98 | 19.64 | +32.3 | **L** (beats c-O0; c-O2 vectorizes) |
| 2 | math **sqrt** | 9.77 | 1.81 | +8.0 | **L** (hardware FSQRT — optimal) |
| 3 | float **leibniz** | 8.05 | 0.90 | +7.2 | **L** (M1 finiteness lever) |

### Excluded / already complete

- **No baseline (mfb-only), regression-track only (235):** the whole `list (Record-*/State-*)`,
  `map (Record-*/State-*)`, `set (Record-*/State-*)` matrix; `math fixed` (28.95),
  `vector fixed` (13.58), `mathpipe finance` (4.75), `mathpipe money` (2.67). Several
  `State-Dynamic` rows are catastrophic — `set (State-Dynamic) set` 1723, `list (State-Dynamic)
  reduce` 2984, `set (State-Dynamic) remove` 62 — the **bug-430 whole-record-rebuild on STATE
  mutation** (`[[resource-state-mutation-is-whole-record-rebuild]]`); they are not scored but
  benefit from **C**/**D**'s in-place mutation and the bug-430 out-of-line-layout work. Track
  for regression.
- **Complete (passes all 4, or ≤ 5 ms):** 178 rows incl. the entire `list (Fixed)` Integer
  matrix (sortBy/window/chunks/groupby/partition/reduce), `datetime civil/iso`, `scalar
  classify`, `dispatch trap`, `map` Fixed/Dynamic get/set/hasKey, `encoding`, `serialize`,
  `bits` (per-op), `crypto` sha512/hmac/pbkdf2/cte, `regexbench compile/capture`, `math
  int/float/matmul`, `arena`, `primes`, `record`.

---

## Task 2 — fix sub-plans (index)

Grouped by **shared root cause** so one fix retires many benchmarks. Ordered by aggregate
priority reach (P1s cleared, biggest offenders first). Each gets its own
`plan-86-<letter>-*.md` if large enough to split (A, C, K are the split candidates).

| Sub-plan | Covers (benchmarks) | Priority reach | Root cause (see body) |
|----------|---------------------|----------------|------------------------|
| **A** String-element native collection lowering | list (Dynamic) groupby/window/sortBy/sort_*/partition/chunks/zip/flatten/findLastIndex; listchurn nested; list (Fixed) sort_rand | ~11×P1 + 1×P3 | plan-64's native list generics are **gated to 8-byte fixed-width**; String falls to the interpreted `.mfb` bodies (`builder_values.rs:765/800/826/849/865`); sort/zip/flatten/findLastIndex have **no** native path at all |
| **B** reduce/reduceRight accumulator | list (Dynamic) reduce (3048), reduceRight (877) | 2×P1 (**biggest ms**) | native `reduce` **never frees intermediate accumulators** (`builder_collection_queries.rs:3398-3416`) → a 3.5× arena penalty on top of the O(n²) `acc & s` concat; reduceRight is interpreted |
| **C** native set-algebra builders + in-place add | set (Fixed) union/toSet/symmetricDifference/intersection/difference; set (Dynamic) union/add | 7×P1 | set algebra is interpreted `FOR EACH … add` (`collections_package.mfb:365-418`) over a **whole-set-copy** `add` (`collection_mutate.rs:391`) → O(n²) |
| **D** map in-place removeKey + String mapValues + native merge | mapchurn churn (165), iterate (14.3), map str_ops (6.0); the removeKey matrix rows | 3×P1 + matrix | `removeKey` has no in-place path → O(N) alloc+copy + **`BUCKETS_READY=0`** poisons the paired probe (`map_mutate.rs:1219`, `collection_buffer.rs:190`); merge deep-copies the base map; String `mapValues` not native |
| **E** borrow read-only collection element | dispatch union (160, P2) | 1×P2 | `collections::get` **deep-copies** every element via `copy_flat_block` even for read-only `MATCH` (`builder_collection_queries.rs:10-23`); Expr-union is freeable-flat, ~4M copies/rep |
| **F** string single-pass / memchr | string case (48.7), slice (36.7), strbuild splitjoin (11.35) | 2×P1 + 1×P2 | split/join/slice copy **byte-at-a-time** (no memchr/word-copy); case_map still 2 byte-passes after F1's ASCII quick-check |
| **G** bounds-check elimination (induction-var get/set) | scalar listchurn (10.6), bignum modmul (19.5)/modexp (10.9), mathpipe memo (11.5) | 1×P1 + 1×P2 + 2×P3 | per-access bounds check on `get`/`set(list, i)` where `i` is a loop-induction var < loop-invariant `len`; the C peers index stack arrays |
| **H** vector op-inlining | vector int (55.7), math (30.9), float (20.9) | 3×P2 | `vector_op_inlinable` inlines length/distance/lerp **Float-only** and `normalize` for **no** type (`builder_vector_inline.rs:104-111`) → FUNC call + arena-block materialize + software isqrt |
| **I** regex compiled handle + replace slice-build | regexbench replace (25.1), alternation (19.1) | 2×P1 (**capped floor**) | recompile-per-call + dual-list makeCtx + O(n) start-restarts (no prefilter for Class/Alt) + replace O(n²) concat; interpreted CPS matcher is a structural floor vs C |
| **J** native csv parse | parse csv (5.01) | 1×P1 (borderline) | interpreted per-scalar state machine over `List OF Integer`; `separatorLength` twice/row; per-cp concat in `__csv_decodeRange` |
| **K** COW / refcount collection buffers + String-element layout | list (Dynamic) copy (12.5), set (14.3), transform (23.6), insert/removeAt (~8.8) | 1×P1 + amplifier | 40-byte header has **no refcount word** → every alias boundary deep-copies (`copy_collection_tight`); a growing String element forces an out-of-line whole-list rebuild per `set` (bug-430) |
| **L** transcendental / float / overflow / formatter kernels (**capped**) | 12 math + sqrt; float nbody/leibniz/mandelbrot; fib; thread sum; io format; crypto sha256 | 14×P2 + 4×P3 + 3×P4 | **capped** by the dd ≤1-ULP no-libm contract, integer-overflow-trap semantics, the intrinsic float formatter, and software-crypto-vs-hashlib-C-backend. Bounded lever = M1 finiteness coalescing (nbody/leibniz/mandelbrot) |

> **Key findings that reshaped the grouping this round** (re-verified at `file:line`):
> - **The `Dynamic` (String) matrix is the dominant new work.** ~11 P1s are one root cause:
>   plan-64's D1–D4 native lowerings are fixed-width-gated (`builder_values.rs:765/800/826/849/865`),
>   so every String list HOF runs the interpreted `.mfb` body. Extending the native paths to
>   String (block-relative-offset-aware copy) retires the whole class (**A**).
> - **`list (Dynamic) reduce` (3048 ms) is the single biggest offender and the highest ROI.**
>   The base O(n²) `acc & s` concat is *structural and fair* (Python does the same fold), but
>   native `reduce` **leaks every intermediate accumulator** (`builder_collection_queries.rs:3398-3416`,
>   plan-26-B) → a 3.5× arena-churn multiplier. Freeing the previous accumulator is
>   semantics-preserving and should collapse reduce toward reduceRight's 877 ms — a cheap,
>   contained, ~2 GB-of-char-copies win (**B**).
> - **`materialize_owned_element` does NOT copy String elements** (`builder_collection_queries.rs:14`
>   excludes `"String"`). So the plan-64 "borrow-on-get" lever (**E**) applies to `dispatch union`
>   (Expr-union *is* freeable-flat) but **not** to the String list HOFs — those pay the
>   interpreted-body cost, not a get-copy. This corrects a plan-64 conflation.
> - **Set algebra is O(n²) by construction** — interpreted re-add loops over a whole-set-copy
>   `add`. Native one-pass builders (single alloc + bulk bucket insert) make each op O(n) (**C**).
> - **bignum is a bounds-check-elimination row, not an algorithm swap** (re-confirmed: the C
>   mirror `main.c:365` is *also* bit-serial). It clusters with memo + scalar listchurn under
>   the shared **G** lever — correctness-critical dataflow, the reason plan-64 left it open.
> - **Math (L), fib/thread (L), io format (L, formatter-capped — L1 was measured noise), and
>   crypto sha256 (L, software-vs-C) remain structural ceilings.** Track for regression.
> Highest leverage: **B** (biggest single row, cheap), **A** (~11 P1), **C** (7 P1), **D**
> (map cluster), then **E/F/G/H**.

### Sub-plan A — String-element native collection lowering (split candidate)

**Covers (~11 P1 + 1 P3):** list (Dynamic) groupby (166), window (88), sortBy (61),
sort_rand/asc/desc (28.9/22.9/22.6), zip (25.6), flatten (24.6), partition (18.7),
chunks (13.6), findLastIndex (11.1); listchurn nested (12.3, native flatten); list (Fixed)
sort_rand (5.07). All min≈median in the log ⇒ genuine, not arena.

**Mechanism.** The bug-430 matrix runs each op over a `Set OF String` / `List OF String`
(`Dynamic`) as well as `Integer` (`Fixed`). plan-64 landed native lowerings for
sortBy/window/chunks/groupBy/partition, but each dispatch gate matches only
`Integer|Float|Fixed|Money` on the monomorph item-type suffix, so String falls to the
interpreted `.mfb` `__collections_*` body (a per-element native call + indirect FUNC
dispatch). Workloads: `benchmark/mfb/src/list.mfb` `test_ld_*` over `buildStrRange(1000)`.

**Root cause (file:line).**
- Native-lowering gates, all requiring 8-byte fixed-width → String excluded:
  `src/target/shared/code/builder_values.rs:765` (sortBy), `:800` (window), `:826`
  (groupBy), `:849` (chunks), `:865` (partition). Comment at `:862-864` states String/Scalar/
  Byte fall through to `__collections_partition`.
- Interpreted `.mfb` fallbacks: `src/builtins/collections_package.mfb:87` (sortBy — copies both
  whole lists per pass via `MUT itemsDst = items`, and String `collections::set` takes the
  out-of-line rebuild branch), `:224` (groupBy), `:280` (chunks), `:298` (window), `:335`
  (partition).
- **No native path at all** (interpreted even for Integer): `sort_asc/desc/rand` (merge sort
  `collections_package.mfb:16`, using String `collections::set` → out-of-line rebuild per move),
  `zip` (`:265`, per-element Pair alloc), `flatten` (`:254`), `findLastIndex` (`:206`).

**Fixes (semantics-preserving — same order/stability/buckets).** A String element is a
block-relative offset into an inlined sub-block (`[[records-inline-their-string-fields]]`), not
a pointer, so the native paths need a **data-region-aware element copy** (append the String
payload to the result's data region and store its offset) rather than an 8-byte word copy.
- **A1 — native `sort`/`sortBy` for String** (biggest: sortBy 61 + sort_* ~74 combined): the
  merge-sort structure D2 already emits; swap the fixed-width `[base+i*8]` load/store for a
  data-region-aware move, or (lower-risk) reuse the generic `lower_list_get`/`set` on the
  ping-pong buffers to kill the per-pass whole-list copy that dominates the `.mfb` version.
- **A2 — native `groupBy`/`window`/`chunks`/`partition` for String**: relax the gates at
  `builder_values.rs:800/826/849/865` and give the nested-block builders (D1/D3/D4) a
  String-payload-copy variant.
- **A3 — add native `flatten`/`zip`/`findLastIndex`** (also lifts the Integer twins): single-pass
  data-region concatenation (flatten), paired data-region copy (zip), reverse scan (findLastIndex).
- Order: A1 (sort family, biggest) → A2 (nested-block ops) → A3 (new lowerings). Correctness
  gate: the per-group `list (Dynamic)` checksums (order-sensitive for the sort rows) unchanged +
  `scripts/artifact-gate.sh` + `tests/` collections + a String-element acceptance fixture per op.

### Sub-plan B — reduce/reduceRight accumulator (highest ROI) — **DONE (B1+B2; B3 deferred to K)**

**Status (landed this session):** B1 + B2 complete and verified. B1 eliminated the native-`reduce`
accumulator/item leak; a focused heavy-fold probe (400-elem `List OF String`, 400 folds) dropped
peak RSS from **869,908,480 → 3,702,784 bytes (~235×)** and wall time from **0.34 s → 0.18 s
(~1.9×)** with byte-identical output (`total=1076000`), measured new-binary vs the pre-change `main`
tip `840945e5f`. B2 made `reduceRight` native (reverse-walk twin of `reduce`, sharing B1's
aliasing-safe reclamation) and deleted the interpreted `__collections_reduceRight` `.mfb` body.
Acceptance fixtures: `tests/rt-behavior/collections/reduce-accumulator-reclaim-rt` (both directions,
every aliasing shape, 500-round UAF stress) and `tests/syntax/collections/func_collection_reduceRight_invalid`;
the pre-existing `hof-string-item-lifetime-rt` still pins the reducer-adopts-item case (`reduce-alias
= fghi`). B3 is **not** in the B execution order (it is conditional and depends on Sub-plan K's
uniquely-owned-mutation infrastructure) — see the B3 bullet. See Corrections.

**Covers (2 P1, biggest ms):** list (Dynamic) reduce (3048), reduceRight (877).

**Mechanism.** `benchmark/mfb/src/list.mfb:1465`: `acc = acc + len(collections::reduce(base, "",
strConcatFn))`, folded 500×; `strConcatFn` (`:204`) is `acc & s` over `buildStrRange(1000)` (final
≈4890 chars). The fold is O(n²) string concatenation — inherent to `acc & s`, and *fair* (Python's
`reduce` does the same, 49 ms). `reduce` is natively lowered (`lower_collection_reduce_call`,
`builder_collection_queries.rs:3321`); `reduceRight` is interpreted (`__collections_reduceRight`,
`collections_package.mfb:160`) with a `MUT acc` local that is scope-drop freed.

**Root cause (file:line).** `reduce`'s native lowering **deliberately never frees intermediate
accumulators** to avoid a UAF when the reducer aliases the item — comment block
`builder_collection_queries.rs:3398-3416` ("the success path likewise leaves intermediate
accumulators unfreed (plan-26-B)"). So ~500k intermediate Strings leak into the arena, adding a
transient-churn penalty on top of the concat — the reason native `reduce` (3048) is **3.5× slower
than interpreted `reduceRight` (877)** doing the identical fold.

**Fixes (semantics-preserving).**
- **B1 (biggest, cheap) — DONE:** free the *previous* accumulator when it is provably not the item and
  not aliased into the new result — i.e. when the reducer's output is a fresh allocation distinct
  from both inputs (the String-concat case). A scope-drop of the superseded accumulator each
  iteration removes the leak; expect reduce → ≈ reduceRight (877) or better. Reuses the escape
  reasoning in `[[nir-visitor-exhaustive-escape-analysis]]`.
  *Landed:* `lower_collection_reduce_impl` now tracks accumulator ownership at runtime (the seed
  starts not-owned so it is never freed / never double-freed) and each success iteration frees the
  superseded item and accumulator, guarded by runtime pointer-equality against the item and the old
  accumulator. Value semantics guarantees a returned String never *partially* aliases an input, so
  pointer equality is an exact, sufficient aliasing test — the bug-307 adopt-item / return-acc cases
  stay safe. The failure path still frees nothing (`emit_callback_failure_exit(None)`); it leaks at
  most one in-flight item/accumulator on the rare error path.
- **B2 — DONE:** native `reduceRight` mirroring `reduce` (with B1's free) — removes the interpreted per-
  element overhead. *Landed:* `lower_collection_reduce_right_call` shares `lower_collection_reduce_impl`
  with a `reverse` flag; new `initialize_collection_loop_slots_reverse` / `advance_collection_loop_reverse`
  walk the cursor from the last element to the first (`reduceRight(xs,i,f) == reduce` over the
  reverse-iterated elements with the same `FUNC(U,T) AS U`). `reduceRight` moved from the source-generic
  `FUNCTIONS` list to `NATIVE_MEMBERS` (descriptor + resolver + `native_builtin_target` +
  `inline_builtin_raw_supported` + both `builder_values.rs` dispatch sites), and the `.mfb` body was
  deleted; its man citations repoint to the native lowering.
- **B3 (structural note) — DEFERRED (depends on K):** the residual O(n²) concat is the fair floor; an
  in-place growing accumulator (append the item's bytes to a uniquely-owned `acc` in place) would make
  it O(n) but **overlaps K** (COW / uniquely-owned mutation). The reducer's `acc & s` is *user* code, so
  making its `&` mutate in place requires K's general uniquely-owned-mutation analysis — infrastructure a
  separate sub-plan owns and this "Sub-plan B only" scope excludes. B3 is explicitly outside the B
  execution order (below) and the plan gates it on "only pursue if B1 leaves a gap to Python"; B's own
  acceptance criterion (checksums + UAF fixture + artifact-gate) does not require Python parity. Track
  with K.
- Order: B1 → B2 (**both landed**). Gate: `list (Dynamic) reduce`/`reduceRight` checksums unchanged + a reducer-
  aliases-item acceptance fixture (prove no UAF) + `scripts/artifact-gate.sh`.

### Sub-plan C — native set-algebra builders + in-place add (split candidate)

**Covers (7 P1):** set (Fixed) union (110), symmetricDifference (70), toSet (69), intersection
(17), difference (17); set (Dynamic) union (7.4), add (5.07). All min≈median ⇒ genuine O(n²).

**Mechanism.** `benchmark/mfb/src/setops.mfb` builds `Set OF Integer` (Fixed, ~300 elems) /
`Set OF String` (Dynamic, ~100). The algebra ops are interpreted source generics, each a
`FOR EACH … collections::add` loop over a native but **whole-set-copy** `add`.

**Root cause (file:line).**
- Interpreted algebra: `src/builtins/collections_package.mfb` `__collections_toSet` (`:365`),
  `union` (`:374`, `result = a` then re-add all of `b`), `intersection` (`:383`), `difference`
  (`:394`), `symmetricDifference` (`:405`, two passes).
- `add` is native (`builder_values.rs:1743` → `lower_set_add`, `collection_mutate.rs:391`) but
  **not in-place**: it `copy_collection_tight`s the whole set (`:425`) then inserts; the copy marks
  buckets not-ready (`emit_reserve_map_buckets`, `:481`), so the insert's first probe rebuilds the
  index O(N). Docstring `:390` flags in-place `MUT` as an unlanded follow-up. So each `add` is O(N)
  → n adds = **O(n²)**. `contains` is a cheap FNV-1a hashed probe (`builder_collection_queries.rs:100`)
  and is *not* the bottleneck.

**Fixes (correctness-neutral — `add` is idempotent/order-stable).**
- **C1 (biggest):** native one-pass set-algebra builders (union/intersection/difference/
  symmetricDifference/toSet): size-hint alloc the single result, build its buckets once
  (`ready=1`), and bulk-insert with hashed `contains` probes — O(n) total instead of O(n²). Covers
  the top 6 rows; add a String-payload variant for the Dynamic twins.
- **C2:** in-place `MUT` `add` fast-path (the `collection_mutate.rs:390` follow-up): when the set
  operand is uniquely owned, skip `copy_collection_tight` and insert directly (amortized O(1)) —
  retires `set (Dynamic) add` and compounds with C1 if the builders reuse an owned accumulator.
  Mirrors the landed list-append in-place path (`[[collection-memory-mgmt]]`).
- Order: C1 → C2. Gate: `set (Fixed)`/`(Dynamic)` checksums unchanged + set acceptance fixtures +
  `scripts/artifact-gate.sh`.

### Sub-plan D — map in-place removeKey + String mapValues + native merge

**Covers (3 P1 + matrix):** mapchurn churn (165), iterate (14.27), map str_ops (5.97); plus the
mfb-only removeKey matrix rows (`map (State-Dynamic) removeKey` 62.6, `State-Fixed` 17.3). All
min≈median ⇒ genuine.

**Mechanism.** `mapchurn churn` (`mapchurn.mfb:38-68`, base 500, 4000 cycles) is dominated by
`removeKey`; `iterate` (`:74-109`, N=1000, 100 passes) by `merge` + keys/values; `map str_ops`
(`map.mfb:100-133`, Map OF String TO String) by String `mapValues` + `merge` + `removeKey`.
`set` is already in-place + hash-incremental; `mapValues` for 8-byte values is native (C2 landed).

**Root cause (file:line).**
- **removeKey — no in-place path.** In-place dispatch (`builder_control.rs:567-581`) covers only
  append/bulk-append/set/prepend/concat; no `try_inplace_remove_key_assign` exists (grep-empty). So
  `m = removeKey(m,k)` → `lower_map_remove_key` (`map_mutate.rs:1219`): O(N) survivor scan (`:1279`),
  fresh `arena_alloc` (`:1351`), O(N) entry copy (`:1395`, per-entry `value_length` bytes at `:1412`),
  and the fresh header **resets `BUCKETS_READY=0`** (`collection_buffer.rs:190`). The next `set`'s
  probe then rebuilds the whole index via `_mfb_rt_map_probe` (`builder_collection_query.rs:274-278,394`)
  — **two O(N) passes per cycle**. The State/record matrix rows scale with total map bytes (the
  `value_length` copy), so a large STATE payload inflates it 26× (`State-Dynamic` 62.6 vs plain 2.4).
- **merge deep-copies base:** `__collections_merge` (`collections_package.mfb:325-333`) opens
  `MUT result = a` → owner-copy deep-copies the whole base map each call (100× for iterate).
- **String `mapValues` not native:** gate `builder_values.rs:788-790` allows only 8-byte
  Integer/Float/Fixed/Money → String falls to `.mfb __collections_mapValues` (`:246-252`), an N-`set`
  rebuild leaving `ready=0`.

**Fixes (value semantics preserved — same survivor set + iteration order).**
- **D1 (biggest):** `try_inplace_remove_key_assign` — delete the entry in place (compact the data
  tail + unlink from its bucket incrementally), keep `BUCKETS_READY=1`, no alloc/copy. Removes both
  O(N) passes of `mapchurn churn` (165 → est. ~40) and the whole-map byte copy of every matrix
  removeKey row. ~80–120 LOC; register-spill discipline across the bucket-maintenance call
  (`[[arena-alloc-clobbers-x14-x15]]`); checksum-catchable. **Measure behind a toggle first** — the
  incremental bucket unlink must not cost more than it saves (the C1-style trap plan-64 flagged).
- **D2:** native String-value `mapValues` (variable-width same-type path — the 8-byte in-place rewrite
  doesn't apply; copy the key/bucket structure, rebuild only value payloads keeping `ready=1`).
- **D3:** native `merge` — size to `|a|+|b|`, copy `a` once with buckets built (`ready=1`), bulk-insert
  both in one pass. Modest (the base copy is inherent to value semantics) — lowest ROI.
- Order: D1 → D2 → D3. Gate: map/mapchurn checksums (catch bucket corruption as a wrong lookup) +
  `cargo test` + map acceptance fixtures + `scripts/artifact-gate.sh`.

### Sub-plan E — borrow read-only collection element

**Covers (1 P2):** dispatch union (160.6).

**Mechanism / root cause (file:line).** `benchmark/mfb/src/dispatch.mfb:44` binds
`LET e = collections::get(nodes, i)` then `MATCH e` (`:45`) read-only. `get` lowers through
`materialize_owned_element` (`builder_collection_queries.rs:10-23`) → `copy_flat_block`
(`builder_collection_layout.rs:370`): a fresh arena copy per element. The element is an `Expr`
union — freeable-flat and ≠`"String"`, so unlike String list elements it **does** hit the copy
(`:14` gate). ~4M copies/rep. MATCH's own variant binding already aliases the inline block without
copying (`builder_control.rs`), so the copy is pure overhead.

**Fixes (semantics-preserving — copy only when the element escapes).**
- **E1:** return an aliasing borrow (pointer into the container's inline element) for a `get` whose
  result is consumed read-only within the statement — MATCH scrutinee, field read, predicate arg;
  copy only when it is stored/returned/mutated (escape analysis, ride
  `[[nir-visitor-exhaustive-escape-analysis]]`).
- **E2:** fuse `MATCH collections::get(list, i)` into a direct read of the inline element's
  tag+payload (no intermediate `e` block).
- Gate: dispatch checksum unchanged + `scripts/artifact-gate.sh`.

### Sub-plan F — string single-pass / memchr (case / slice / split / join)

**Covers (2 P1 + 1 P2):** string case (48.7), strbuild splitjoin (11.35), string slice (36.7). All
genuine/linear.

**Mechanism / root cause (file:line).** The whole strings family copies **byte-at-a-time** through
`emit_materialize_string_from_bytes` (`builder_collection_layout.rs:2287-2295`, load_u8/store_u8) and
the inline split/join loops.
- **case:** `lower_strings_case_map` (`builder_strings_builtins.rs:461`) — F1's ASCII quick-check
  landed (`:509-572`) but it is still **two byte-level passes** (scan `:521-529` + transform `:559-568`);
  8 ops × 50000 = 400k allocs.
- **split/join:** `lower_strings_split` (`:1603`) is a 2-pass byte-at-a-time delimiter scan (length
  `:1686-1722`, write `:1801-1827`); `lower_strings_join` (`:1394`) byte loops (`:1552-1582`). No
  memchr / word-copy.
- **slice:** same split+join+`lower_mid` (`builder_search.rs:653`, ASCII fast-forward `:812` but
  byte-copy `:910-918`) family, 9 ops × 50000.

**Fixes (bounded, ~2×; op-count/allocation-bound).**
- **F1 (done):** case_map ASCII quick-check (landed plan-64).
- **F2:** memchr single-byte delimiter scan + 8-byte word-at-a-time block copy in split/join/mid;
  fuse split's two scans for a single-char delimiter.
- **F3:** collapse case_map to a single pass (one over-allocate-to-byte-len write).
- Gate: string/strbuild checksums + `scripts/artifact-gate.sh`.

### Sub-plan G — bounds-check elimination on induction-var get/set

**Covers (1 P1 + 1 P2 + 2 P3):** scalarbench listchurn (10.6), bignum modmul (19.5)/modexp (10.9),
mathpipe memo (11.5).

**Mechanism / root cause (file:line).** Each hot loop does `collections::get`/`set` on a
**loop-invariant-length** list indexed by a **loop-induction variable**, paying a per-access bounds
check the C peers (stack arrays) and Python don't.
- **bignum:** `benchmark/mfb/src/main.mfb:696-745` `bnMod` — the `k`/`kk` shift-and-subtract inner
  loops over `r` (`k < rlen`, `rlen = len(r)`). The C mirror `benchmark/c/main.c:365` `bn_mod` is
  **also bit-serial** (header `:313` "schoolbook mul + bit-serial mod"), so a Barrett/limb rewrite
  would be an *unfair algorithm swap* — the only fair lever is the bounds-check elision (reclassified
  from plan-64 K).
- **memo:** `benchmark/mfb/src/mathpipe.mfb:191` `get(ways,a)+get(ways,a-c)` + `set(ways,a)` (`:192`),
  `ways` length loop-invariant, `a` induction.
- **scalar listchurn:** `benchmark/mfb/src/scalarbench.mfb:155` ascent loop
  `get(scalars,i) < get(scalars,i+1)` dominates.

**Fixes (semantics-preserving but correctness-critical dataflow).**
- **G1:** emit an unchecked `get`/`set` when a dataflow pass proves `0 ≤ index < len` from the
  induction bound and a loop-invariant length. An unsound elision is **silent memory unsafety**, not a
  checksum-catchable wrong answer — this is why plan-64 left it open despite naming it the shared
  lever. Needs the `.ai/compiler.md` register-lifetime + verification bar.
- **G2 (memo only):** strength-reduce the constant `MOD 1000000007` to a conditional subtract (sum of
  two values each < m is < 2m).
- Gate: bignum/memo/scalar checksums + overflow behavior + `scripts/artifact-gate.sh` + a targeted
  out-of-bounds negative test proving the elision does not fire when the bound is not provable.

### Sub-plan H — vector op-inlining

**Covers (3 P2):** vector int (55.7), math (30.9), float (20.9). Also lifts `vector fixed` (mfb-only).

**Mechanism / root cause (file:line).** `vector_op_inlinable` (`builder_vector_inline.rs:104-111`):
`scale`/`dot` inline all types (`:106`), `cross` for 3D (`:107`), but `length`/`distance`/`lerp`
inline **Float-only** (`:108`) and `normalize` is **absent from the match** → no type inlines it.
Non-inlined ops make a `#vector_<op>` FUNC call and materialize the register-native operand to a fresh
N×8 arena block (`vector_value_as_block:184`); Integer/Fixed `length` also runs software isqrt
(`vector_package.mfb:302 → __vector_isqrtFloor:137`). `vector math` is dominated by `normalize` ×2/iter
(`vector.mfb:23-24`).

**Fixes (semantics-preserving — reuse the module's bit-identity lower_value technique).**
- **H1 (biggest, vector math):** inline `normalize` as `scale(v, 1/length(v))` with the zero-length
  `FAIL error(77050002)` guard — blocked today because the guard is control flow the pure-expression
  inliner can't emit; needs a guard-capable inline path.
- **H2 (vector int/fixed):** relax the `element=="Float"` clause at `:108` for length/distance/lerp,
  feeding register-native lanes to skip the block materialize and inlining the Fixed/Integer isqrt
  (`emit_fixed_sqrt` already deterministic).
- Gate: `tools/math-kernels/runtime_ulp.py` (normalize reuses gated `math::sqrt`) + scalar-vs-array
  bit identity + vector checksums + `scripts/artifact-gate.sh`.

### Sub-plan I — regex compiled handle + replace slice-build (capped floor)

**Covers (2 P1, capped):** regexbench replace (25.1), alternation (19.1).

**Mechanism / root cause (file:line).** `src/builtins/regex_package.mfb`: `__regex_replace` (`:1869`)
recompiles per call (`:1870`), `makeCtx` builds a dual `List OF String` + `List OF Integer` (`:229-236`),
and the replace loop does `strings::mid` re-walks + `out & …` **O(n²) concat** (`:1876-1880`). Patterns
`[0-9]+` (Class) and `cat|dog|…` (Alt) get `requiredFirstCp = -1` (`:1928-1944`), so plan-77 R5's
first-scalar prefilter (`:868-877`) does **not** fire → `__regex_searchFrom` (`:864`) runs the full
interpreted CPS matcher (`matchNode:609`, `matchAlt:668`) at every start position × N matches = O(n²).

**Fixes (source-package; checksums unchanged; a structural floor remains).**
- **I1:** a compiled-pattern handle so compile/find/replace parse once and reuse the program (retires
  the recompile; helps `regexbench compile`).
- **I2:** build `replace` output from `ctx.chars` slices instead of `out & …` (kills the O(n²)
  accumulation) and hoist `toScalars(repl)` out of the match loop.
- **Structural floor:** an interpreted CPS backtracking matcher over `List OF …` cannot reach C POSIX-NFA
  / CPython `re` (25 ms → 0.03 ms). I1/I2 are large constant-factor wins; **do not gate on parity**.
- Gate: regexbench/parse checksums + match counts + `scripts/artifact-gate.sh`.

### Sub-plan J — native csv parse

**Covers (1 P1, borderline):** parse csv (5.01, min 4.98 ⇒ linear).

**Mechanism / root cause (file:line).** `__csv_parse` (`csv_package.mfb:34`) is an interpreted
per-scalar state machine over a `List OF Integer`: per-scalar `collections::get` (`:50`),
`separatorLength` called **twice per row** (`:65`+`:73`), list appends, and per-cp `out & fromCodepoint`
in `__csv_decodeRange` (`:140`). A3-csv (no intermediate list, `:129`) already landed; the residual is
the interpreted scan (~15× C). **Not arena.**

**Fixes.**
- **J1:** a native byte-level csv-parse builtin (or hoist `separatorLength` to once/row and batch the
  decode). Borderline row (5.01 ≈ the 5 ms complete bar) — low priority; pursue only if it regresses.
- Gate: `parse csv` checksum 6003000 unchanged + csv acceptance fixtures.

### Sub-plan K — COW / refcount collection buffers + String-element layout (split candidate)

**Covers (1 P1 direct + broad amplifier):** list (Dynamic) copy (12.5), set (14.3), transform (23.6),
insert/removeAt (~8.8); amplifies A's sort/groupBy buffer copies and B's reduce accumulator.

**Mechanism / root cause (file:line).** The 40-byte header (`error_constants.rs`) has **no
refcount/version word** and there is no COW in codegen: `lower_value_owned` (`builder_values.rs`)
unconditionally deep-copies any aliasing source via `copy_collection_tight`
(`builder_collection_layout.rs:426`). `list (Dynamic) copy` (`copyStrs` returns its arg) is a full
String-list memcpy per call. `list (Dynamic) set` grows a String payload → the out-of-line whole-list
rebuild branch (bug-430). `transform` is already native but String-allocation-bound. `insert`/`removeAt`
are native with **inherent** O(n²) data-region reflow (repeated `insert(0)`) — not COW-fixable.

**Fixes.**
- **K1 (interim, contained):** move-elision escape analysis — `RETURN local` of a caller-dead value
  becomes a move; the identity shape `FUNC f(xs) RETURN xs` (copyStrs) elides the copy entirely. Retires
  `list (Dynamic) copy` cheaply.
- **K2 (large):** add a refcount/version word + copy-on-write to `copy_collection_tight` — share on
  RETURN/param-alias, split on first mutation. Makes A/B/D's buffer copies free too. Largest design
  change in the plan (value model + every mutation path); semantics must stay observably value-copy —
  **defer** until A/B/C/D cut copy volume; reassess.
- **K3:** an out-of-line String-element list layout so a growing String `set` need not rebuild the whole
  list (the bug-430 in-place-mutation follow-up).
- Order: K1 early (contained), K3 with A, K2 deferred. Gate: value-semantics fixtures (`tests/`) +
  `scripts/artifact-gate.sh`.

### Sub-plan L — transcendental / float / overflow / formatter kernels (structurally capped)

**Covers (14 P2 + 4 P3 + 3 P4 + 2 P1):** math sin/cos/tan/exp/log/log10/pow/simd/asin/acos/atan/atan2,
sqrt; float leibniz/nbody/mandelbrot; recurse fib; thread sum; io format; crypto sha256.

**Mechanism + ceiling (file:line).**
- **Transcendentals + sqrt:** all `Float` math is native inline f64, no libm; scalar and array share one
  path so ULP/bit-identity is enforced by construction. Each open-codes a **double-double compensated
  Horner** to meet the ≤1-ULP / deterministic / no-libm contract: `emit_tan_body`
  (`builder_simd_float_math.rs:1352`), `emit_sin_cos_body` (`:1115`), `emit_exp_body` (`:1498`),
  `emit_log_body` (`:1636`), `emit_atan_core` (`:679`), `emit_compensated_horner` (`:1853`); pow is a
  software fdlibm port (`builder_pow.rs:169`). `sqrt` is a single hardware `float_sqrt_d`
  (`builder_math.rs:1101`) — IEEE-exact, optimal. The 3–10× gaps are the structural dd-vs-libm delta;
  achievable semantics-preserving gain ≈ 0.
- **fib/thread:** mandatory checked add under integer-overflow-trap semantics —
  `emit_integer_binary_checked` (`builder_numeric.rs:828`); `total + i` / `fib()+fib()` are
  overflow-capable (non-elidable, `:776`), so the check stays.
- **io format:** the concat chain is cheap; the cost is the intrinsic `float_format.rs` per-value
  formatter (plan-64 L1 concat-fusion was built and measured as noise). Formatter-capped.
- **crypto sha256:** software `bits` core (`crypto_hash.mfb`, rotr32/band/sr) vs Python hashlib's C
  backend — structural, same shape as the math band.

**Fixes (the only bounded lever).**
- **L1 (M1):** coalesce sibling finiteness checks at a shared boundary — nbody/mandelbrot's `nzr`/`nzi`
  are two producers per iteration; a combined `fmax(|nzr|,|nzi|)` vs +Inf halves the branch count
  (bit-identical trap set; keep the earliest `line:char` stamp). Bounded — tens of percent on
  nbody/leibniz, not a multiple. mandelbrot already beats c-O0 (loses only to c-O2 autovectorization).
- **BLOCKED:** the transcendental band, fib/thread, io format, and crypto sha256 cannot reach their
  bands without breaking the dd-precision / overflow-trap contract, replacing the float formatter, or
  swapping the software crypto core for a C backend. **Documented as ceilings; track for regression.**
- Gate: `tools/math-kernels/runtime_ulp.py` + scalar-vs-array bit identity + all math/float/crypto
  checksums unchanged.

## Validation Plan (all sub-plans)

- **Correctness first:** every fix produces identical observable output — the benchmark checksums on
  stderr (`csv=6003000`, `json=5000`, `regex=200`, `sha256=320768`, plus each group's printed checksum:
  the `list`/`map`/`set` matrix per-container checksums, mapchurn, string/strbuild, scalar, dispatch,
  vector, bignum, math/float, io) **unchanged** — and passes `scripts/test-accept.sh` + `tests/`. No
  language/semantic/syntax/precision change; value-semantics and integer-overflow-trap semantics
  preserved.
- Re-measure the affected group with the **same `--run 10`** as the source logs (`20260804-083935`);
  confirm the row's band improved (ideally to complete). The full `--run 10` suite is slow — use a
  throwaway trimmed `benchmark/mfb` copy (`main()` cut to the target group) built with the current
  compiler for the per-fix loop, and one full clean run at finalization.
- Codegen changes: `scripts/artifact-gate.sh` (byte-deterministic 4-target self-diff — do NOT run
  concurrently with another gate, `[[no-concurrent-artifact-gate]]`). Math changes:
  `tools/math-kernels/runtime_ulp.py`.

## Corrections

- **Sub-plan B (B1+B2) landed this session; B3 deferred to K.** The plan framed B1 as "free the
  *previous* accumulator … via a scope-drop / `[[nir-visitor-exhaustive-escape-analysis]]`". As
  implemented it is a **runtime** reclamation, not a compile-time escape analysis: because the reducer
  is an opaque function value, whether its output aliases the item or the accumulator is only knowable
  at run time, so the lowering emits pointer-equality guards (`new == item?`, `new == old_acc?`) plus a
  per-accumulator ownership flag (seed = not-owned). This is stronger than the plan's "provably distinct"
  wording (which implied a static proof) and is what makes the bug-307 adopt-item case safe without a
  UAF. B2's `reduceRight` migration from source-generic to native touched more seams than "mirror reduce"
  implied — descriptor authority, resolver, `native_builtin_target`, `inline_builtin_raw_supported`, two
  `builder_values.rs` dispatch sites, `.mfb` body deletion, and man-citation repointing (the
  `man_citations_resolve` unit test caught the dangling `__collections_reduceRight` citation). No scope
  was re-split. B3 was reclassified from "conditional structural note" to **blocked on Sub-plan K**: an
  in-place growing accumulator needs K's uniquely-owned-mutation analysis to make the *user* reducer's
  `acc & s` mutate in place, which "Sub-plan B only" excludes — recorded, not silently skipped.
- **Acceptance baseline: 5 pre-existing reds, none from Sub-plan B.** The full `scripts/test-accept.sh`
  on the merged tree reports 5 mismatches — `rt-behavior/tls/tls-connect-google-rt` (network flake),
  `syntax/app/macos-app-mode-term` (`.ncode` path), and three poll-list diagnostic goldens
  (`syntax/net/func_net_poll_invalid`, `syntax/tls/poll_invalid`, `syntax/csv/func_csv_parse_invalid`,
  each stale by a prior `net`/`tls` poll-list overload adding `or List OF RES Socket, Integer` to the
  expected-arguments string). All 5 were reproduced red at the fork base `2c50e5955` with a
  base-built binary before any Sub-plan B commit, proving they are pre-existing and outside collections
  (Sub-plan B touches only `collections`). `scripts/artifact-gate.sh all` is fully green (0 diffs) and
  the collections behaviour/diagnostic fixtures all pass; the 5 reds are the standing baseline
  (`[[acceptance-preexisting-reds-baseline]]`), not a regression.

## Open Decisions

- **B (reduce leak) is the highest-ROI single fix** — biggest offender (3048 ms), cheap and contained
  (free the superseded accumulator). Recommend landing B first, then **A** (~11 P1, the String matrix)
  and **C** (7 P1 set algebra). Decision: **B first**, then A, C, D, then E/F/G/H.
- **A vs K ordering for the String sort family.** A1 (native String sort) and K's COW both attack the
  per-pass whole-list copy. Recommend A1 via the lower-risk generic-get/set-on-ping-pong-buffers variant
  (kills the per-pass copy without the header rewrite); take K2 (refcount COW) only if the residual
  amplification still dominates. Decision: A1 self-contained; K2 deferred.
- **G (bounds-check elimination) is correctness-critical and shared across 4 rows** (bignum/memo/scalar).
  An unsound elision is silent UAF, not a wrong checksum. Recommend a conservative dataflow pass gated to
  provable `induction < loop-invariant len`, with a negative test proving it does not fire otherwise.
  Decision: pursue G as one lever for all four, held to the `.ai/compiler.md` bar.
- **M/L (math), fib/thread, io format, crypto sha256 are structurally capped** — cannot reach their
  bands without breaking a contract. Decision: ceiling accepted; land only the bounded L1 (finiteness
  coalescing) if cheap/non-gating.
- **I-regex has a structural floor** — a source CPS matcher will not match C/CPython; I1/I2 are large
  constant-factor wins but do not gate on parity. Decision: pursue I1 + I2; bound the expectation.
- **No-baseline `State-Dynamic` catastrophes** (set/list STATE mutation, up to 2984 ms) are the bug-430
  whole-record-rebuild — out of this plan's scored scope but the same in-place-mutation levers (C2/D1/K3)
  retire them. Decision: track for regression; fold into C/D/K where the in-place path lands.
