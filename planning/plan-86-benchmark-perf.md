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

### Sub-plan bodies — split into per-sub-plan files (2026-08-08)

The mechanism/root-cause detail, the **fixes checklist (`[ ]`/`[x]`)**, per-sub-plan acceptance, and
Corrections now live in one file per sub-plan (each independently landable and `/follow-plan`-trackable).
The index table above stays as the one-line overview; open a file for the checklist:

- **A** → [plan-86-A-string-native-lowering.md](plan-86-A-string-native-lowering.md) — **DONE** (findLastIndex P3-clear 11.18→5.66; **groupBy 162→0.366 COMPLETE**, ~445×; chunks/window/zip marginal/capped but correct; partition/sortBy/sort/flatten prior). String-KEY groupBy is a non-scored later extension.
- **B** → [plan-86-B-reduce-accumulator.md](plan-86-B-reduce-accumulator.md) — **DONE** (B1+B2; B3 → K)
- **C** → [plan-86-C-set-algebra.md](plan-86-C-set-algebra.md) — **DONE** (C2 in-place add retired all 7 P1; C1 moot)
- **D** → [plan-86-D-map-ops.md](plan-86-D-map-ops.md) — **D1 removeKey DONE** (in-place entry compaction, mapchurn churn 161→22 ms ~7.3×; stays P1); D2 mapValues (modest ~5.97ms) / D3 merge open
- **E** → [plan-86-E-borrow-element.md](plan-86-E-borrow-element.md) — **DONE** (E1 read-only get-borrow: **dispatch union 160.9 → 43.2 ms, ~3.7×**; classifier follows the `MATCH e` → `$matchN=e; MATCH $matchN` desugar, borrows both; fixed the plan-25 pending-temp free-into-container corruption; E2 moot/subsumed). Still P2 (interpreted MATCH/tree-eval residual).
- **F** → [plan-86-F-string-single-pass.md](plan-86-F-string-single-pass.md) — open, **fully mapped** (F2 word-copy via existing `emit_block_copy_advance` + SWAR memchr = tractable/near-zero-risk but MARGINAL ~1.3–1.8×, short strings are alloc/call-bound; F3 case_map single-pass higher-risk). F1 landed plan-64.
- **G** → [plan-86-G-bounds-check-elim.md](plan-86-G-bounds-check-elim.md) — open, **fully mapped** (correctness-critical, UAF if unsound). **HONEST SCOPING: the SAFE minimal G1 cut helps ONLY scalar listchurn (1 P1, 10.6ms)** — memo/bignum do NOT match the `FOR i=0..len(L)-1`/un-reassigned-L shape (memo bound is a const + `ways` reassigned by `set`; bignum uses WHILE + `set`-reassigned) and need a strictly larger symbolic-range pass. Reuse plan-39 I1's range-fact substrate + `scan_loop_locals` + conservative-default-false; MANDATORY negative fixtures.
- **H** → [plan-86-H-vector-inline.md](plan-86-H-vector-inline.md) — open, **fully mapped**. **H2 TRACTABLE (do first)**: relax the Float-only gate (`builder_vector_inline.rs:108`) for length/distance ONLY (keep lerp Float-gated), extend the rewrite branches (Fixed→math.sqrt, Integer→`__vector_isqrtRound` call) → removes operand block-materialize, helps vector int (55.7ms, double-digit-% cut) + vector fixed. **H1 HARDER (guard-capable normalize inline, new statement-emitting machinery)**: the only lever for vector math (30.9) + vector float (20.9). Bit-identity mandatory.
- **I** → [plan-86-I-regex.md](plan-86-I-regex.md) — **DONE (both moot, measured).** I1 (compile handle) negligible
  (~0.03ms/compile → ~1% of the 25ms replace row; `compile` row already complete) + a public-API change; I2
  (replace output slice-build) implemented→measured 24.8→26.1ms (no win, slightly worse) → REVERTED: the
  `out & …` accumulation is already O(n) in-place append (premise false). Root cause: the prefilter-less CPS
  matcher (`searchFrom` per position for Class/Alt, `requiredFirstCp=-1`) is ~24.6ms of the 25ms — the capped
  structural floor the plan accepted; the only real lever is a Class/Alt first-scalar prefilter, bounded out.
- **J** → [plan-86-J-csv-parse.md](plan-86-J-csv-parse.md) — open (borderline, low priority)
- **K** → [plan-86-K-cow-layout.md](plan-86-K-cow-layout.md) — open (K1/K2/K3; also owns B3)
- **L** → [plan-86-L-transcendental-capped.md](plan-86-L-transcendental-capped.md) — capped (only L1 is a live lever)

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

- **Session (worktree-P-86) summary + prioritized roadmap for the resume.** Landed this session (all
  cargo-test-green + artifact-gate all 0-diff + native/`.mfb` byte-identical): **sub-plan A COMPLETE** —
  findLastIndex 11.18→5.66 (P3 clear), **groupBy 162→0.366 (~445×, COMPLETE)**, chunks 13.4→12.98 / window
  88.7→84.5 / zip 24.7→23.1 (correct but MARGINAL/capped — their `.mfb` already uses native slice/append);
  **D1 removeKey 161.5→21.96 (~7.3×, in-place entry compaction)**. The **META-LESSON**: the two BIG wins
  (groupBy, removeKey) both came from `.mfb` bodies doing a **per-element whole-CONTAINER copy** (map get/set
  over a big bucket; fresh-map rebuild) → native inline mutation makes them O(N). **To find the next big wins,
  look for `.mfb`/lowering paths that copy a whole map/list/record per element** (see the State-Dynamic matrix
  rows — `set (State-Dynamic) set` 1723, `list (State-Dynamic) reduce` 2984 — the bug-430 whole-record rebuild
  is exactly this class). Marginal (constant-factor-only) rewrites where the `.mfb` already uses efficient
  native primitives are NOT worth the surface. **Remaining, prioritized:** D2/D3 DEPRIORITIZED (marginal —
  `.mfb` in-place set already fires). **ALL FOUR BAND-CLEARERS ARE NOW FULLY MAPPED with tractable specs +
  edit points (see each sub-plan doc) — the resume can implement directly:** **E** (borrow read-only element,
  dispatch union 160 P2 — TRACTABLE big win, rides the existing `aliases_union_variant` no-copy/no-free
  discipline; UAF if the copy-skip + cleanup-skip aren't gated on the SAME set); **F** (string memchr/word-copy
  — TRACTABLE near-zero-risk but MARGINAL ~1.3–1.8×, short strings are alloc/call-bound); **G** (bounds-check
  elim — TRACTABLE but the SAFE cut helps ONLY listchurn, correctness-critical UAF, mandatory negative
  fixtures; memo/bignum need a bigger symbolic-range pass); **H** (vector inline — H2 relax length/distance
  gate = tractable, helps vector int; H1 guard-capable normalize = harder, only lever for vector math/float).
  **Recommended resume order by risk-adjusted ROI: E (big, tractable) → H2 (tractable, vector int) → G-listchurn
  (careful) → F (marginal, only if cheap) → H1 (harder) → K1 move-elision (`list (Dynamic) copy`).** Capped/
  track (per plan): I (regex floor), J (csv borderline), K2 (COW — defer), L (transcendental ceilings; only
  L1 live). The two big wins this session (groupBy, removeKey) confirm: hunt O(container)-copy `.mfb` bodies
  (the State-Dynamic matrix rows are the next such class).
- **Sub-plan C: C2 (in-place set `add`) alone retired all 7 P1 rows; C1 (native builders) is unnecessary.**
  The plan ordered C1 (native one-pass builders) first as "biggest" and C2 (in-place add) second. In
  practice the O(n²) was **entirely** the whole-set copy inside `add` — the interpreted
  `FOR EACH … result = add(result, x)` bodies are already the right algorithm, just quadratic because each
  `add` copied. Making `add` in-place (the C2 sibling of the landed list-append-in-place path, ~80 LOC)
  dropped every set-algebra op to ≤ 1.1 ms (union 110 → 0.69), clearing the "≤ 5 ms = complete" bar for
  all 7 P1s. So C1's separate native builders would be pure redundancy — **proven** unnecessary by
  measurement, not skipped on a hunch. The set (Fixed)/(Dynamic) `State-*` matrix rows (whole-record
  rebuild) still stand; C2 helps their `add` but the STATE-mutation cost is the bug-430 residual.
- **Sub-plan A3: the `flatten` benchmark row is `chunks`-bound, not `flatten`-bound.** The scored row is
  `len(flatten(chunks(base, 10)))` — it first builds 100 nested String lists via the **interpreted**
  `collections::chunks` (String `chunks` is not native), then flattens. Native `flatten` (inline-inner
  pointer + `bulk_append`, no per-inner copy) is correct and taken (min 24.2 → 21.7, beyond noise) but the
  row barely moves because `chunks` dominates. So `flatten` alone does NOT clear its P1 row — it needs
  native String `chunks` (a remaining nested-block piece) first. Native flatten still lifts `listchurn
  nested` (Integer flatten on a genuine nested list) and any flatten-heavy code, and is a correct building
  block. Recorded so the next session pairs `chunks` + `flatten` rather than re-measuring flatten alone.
- **Sub-plan A1: `sort` had NO native path for ANY type; native String `sort` now clears G1 for asc/desc.**
  The plan's A note ("`sort_asc/desc/rand` … no native path at all") applied to every element type, not
  just String — `collections::sort` always ran the interpreted `.mfb` merge. Native String `sort` reuses
  the sortBy index-permutation + gather, but with a lexicographic byte compare of the two source Strings
  in the merge (there is no key), and a new `reserve_integer_index_list` to size the `n*8` index buffers
  from the count (sortBy borrowed that sizing from its `transform`-built keys; `sort` has none). Result:
  `sort_asc` 23.12→2.94, `sort_desc` 22.61→2.82 (both now **beat** Python 3.15), `sort_rand` 29.56→4.53
  (close to Python 3.15). Correctness pinned by the identical `871130213` checksum across all three input
  orderings (a wrong sort would diverge) plus the `sort-string-gather-rt` fixture. **Fixed-width `sort`
  followed** (same session): `Integer`/`Fixed`/`Money` route to the same merge with a signed word compare
  + word-copy gather (String path byte-identical), retiring `list (Fixed) sort_rand` (5.07 → 0.99 ms, now
  beats Python 1.85). `Float` stays interpreted (NaN ordering).
- **Sub-plan A1: `sortBy` for String uses an index-permutation gather, not an in-place String merge.**
  The plan's A1 offered two routes (data-region-aware element move, or generic get/set on ping-pong
  buffers). As implemented it is a third: sort an **Integer index permutation** with the existing
  fixed-width word-merge untouched, then gather the Strings once. This keeps the hot merge byte-identical
  and confines all String handling to `transform` (keys) + one gather pass — no change to the merge core,
  no new element-copy primitive. It required (a) building keys via native `transform` rather than a manual
  fill, because a manual `lower_reserved_list(source)` on a String source sizes the keys/index buffers
  from the source's *string-byte* data region, far smaller than the `n*8` the merge writes → an overflow;
  reserving the index buffers from `keys_slot` (a `List OF Integer`, data region `n*8`) fixes the size.
  **Gotcha caught by measurement:** the dispatch gate first required *every* arg re-eval-safe, but the
  keyFn is a `functionRef` NirValue (not Local/Const/Global/LocalRef), so String sortBy silently kept
  taking the `.mfb` path — the benchmark showed ~no change (60 ms) until the gate was narrowed to guard
  only the *source* (args[0]); the keyFn is a pure pointer load. This is a reminder that "correct output"
  did **not** prove the native path ran (the `.mfb` fallback is also correct); only the 61→8 ms drop did.
  Result: `list (Dynamic) sortBy` 61.38 → 8.13 ms; still above Python (3.22) because keys+gather each walk
  the n Strings once (a fuse is possible but out of A1 scope).
- **Sub-plan A: `partition` landed natively for String; it does NOT clear G1 (record-copy floor).**
  The plan grouped `partition` with the "~11 P1" that native lowering would retire. In practice the
  native String path (relax gate + free-after-append, mirroring the already-String-correct `filter`)
  cut it 18.79 → 13.68 ms but left it **above** Python's 8.31 ms, because `partition` returns a
  `Partition OF String` **record** whose two `List OF String` fields are inlined as owned byte copies
  (value semantics) — a second full copy of every element on top of the two output-list allocations,
  which `filter` (single list, no record) does not pay. So `partition` is a real ~27 % win but a
  **G2-class** result, not a G1 clear; fully closing it would need move-into-record (elide the record
  inline-copy) which is K-territory (uniquely-owned move), outside A2. Recorded, not silently dropped.
  The rest of A (sort family, window/chunks/groupBy String, flatten/zip/findLastIndex) remains open and
  genuinely needs the data-region-aware element move — no more "just relax the gate" wins there
  (`filter`/`partition` were the only native bodies already built on the generic String-correct
  get/append primitives; the others use raw 8-byte word-copy loops or have no native path).
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
