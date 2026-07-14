# plan-39: Benchmark performance — close the gap to C/Python

Last updated: 2026-07-14
Effort: xlarge (multi-day, many independently-landable sub-plans)
Platform under test: **aarch64 / macOS** (the target for this work)

## Implementation progress (2026-07-14)

Landed and verified (full acceptance 942 + artifact-gate 0-diff + benchmark
checksums unchanged where applicable):

- **F1 (io write)** — DONE. `fs::setBuffered(f, TRUE)` in `writeLinesFile`
  (`benchmark/mfb/src/main.mfb`). Harness parity fix.
- **A1 (sort/sortBy)** — DONE. Stable bottom-up merge sort in
  `collections_package.mfb`; sortBy carries items+keys in parallel. O(n log n).
- **A2 (any/all/find*/partition)** — DONE. Dropped the `transform` flags list;
  single-pass inline predicate with short-circuit restored.
- **A3 (reduceRight)** — DONE. Backward fold, no reversed copy.
- **H1 (mapValues)** — DONE. Iterates the map directly (FOR EACH) instead of
  building ks/vs/us intermediate lists. (merge already lean.)
- **D (bignum)** — DONE (D2-style, dominant lever). `bnMod` rewritten to run the
  same bit-serial shift/compare/subtract **in place** on one preallocated
  `len(m)+1`-limb buffer — bit-identical result (cross-checked old vs new:
  1360464289 match), ~300k allocs/run → ~O(bnMod-calls). Barrett (D1) not needed.
- **E1 (unicode case ASCII)** — DONE. ASCII fast path in `lower_strings_case_map`
  (both count + write passes) — cp<0x80 skips the 11-deep table search, ±32 range
  map. Bit-identical (a-z/A-Z only ASCII casing).
- **E2 (NFC quick-check)** — DONE. Pure-ASCII pre-scan in
  `lower_strings_normalize_nfc` returns a plain byte copy (ASCII is already NFC),
  skipping decompose/reorder/compose. Differential-tested compose/keep/mixed.

Also landed:
- **G (csv only)** — DONE. `__csv_parse` scans Unicode scalars
  (`encoding::utf32Encode`) and builds fields with `encoding::utf32Decode`, no
  grapheme List-OF-String materialization, no O(n²) concat. csv 20→8.5 ms,
  checksum 6003000 unchanged. json/regex **deferred** (json threads
  `List OF String` through 15+ functions; regex is structurally heavy).
- **C1/C2 (vector)** — DONE. C1 inlines scale/dot/cross for Fixed/Integer
  (bit-identical). C2 seeds `__vector_isqrtFloor` from the hardware Float sqrt +
  overflow-safe division-only correction (exact floor; 0 mismatches vs Newton
  over 100k values + boundaries + near Integer-max). vector int 99→57 ms.
  C3 is moot for the int path (`length` stays a FUNC). vector fixed 14.6→14.0.
- **A4 (native slice)** — DONE for window/chunks/take/drop. `try_inline_slice_op`
  intercepts the internal `#collections_slice$T` helper (window/chunks) and emits a
  native bulk range copy (adapts `lower_map_projection`'s byte-wise payload copy +
  running offset; correct for every element type; start/stop clamped to [0,count]).
  take/drop delegate to `__collections_slice`. window 203→117, chunks 30→15,
  take 10.5→4.2 (COMPLETE), drop 10.8→4.3 (COMPLETE). Verified value-independence +
  Integer/String/nested + all edge cases; accept 942, gate 0-diff, checksums same.
  zip NOT done (builds Pair records, not a slice).

### Measured result (release mfb, `--run 10`, same metric as source logs)

Rows moved to **COMPLETE (≤5 ms)**: io write 26.7→1.7, any 5.5→1.0,
all 5.3→1.0, findIndex 13.8→2.6, findLastIndex 13.8→2.5, reduceRight
23.5→2.8, take 10.5→4.3, drop 10.8→4.4, **zip 7.6→1.8**. Rows that now **beat Python**
(cleared P1→P2): bignum modmul 228→23.2, modexp 123→12.9. Large in-band gains:
sortBy 647→69, string case 155→68, csv 20→8.5, partition 18.4→14.6, window
203→117, chunks 30→15, vector int 99→57, vector fixed 14.6→14.0. Remaining P1s
needing more work: window/chunks/zip (native slice landed for window/chunks;
zip still source), sortBy/case/csv/partition (much faster but still lose to
Python by construction).

### Additional findings (2026-07-14, second pass)

- **zip (A4)** — DONE. `try_inline_zip_op` builds `List OF Pair$A$B` natively when
  A/B are both fixed-width scalars (Pair is a flat 16 bytes `[a@0][b@8]`); String/
  record pairs fall back to the FUNC. zip 7.6→1.8 ms (COMPLETE). **The whole A
  sub-plan is now complete** (window/chunks/take/drop/zip all native).
- **B2 (sqrt d-native)** — DONE. `lower_math_sqrt` reads the Float operand into a
  `d` register (`operand_as_double`, no GPR shuttle), `fcmp`/`fsqrt`, returns the
  `%fN` result d-native. Bit-identical (sqrt checksum 2980093.768938). Committed.
- **G json/regex do NOT benefit** — investigated and rejected. `strings::graphemes`
  is a **native O(n)** op; replacing it with a source scalar loop
  (`utf32Encode`+per-cp `utf32Decode`) regressed json ~1000× (6→5649 ms — the
  source per-char loop is far slower than the native splitter). csv was the only
  real parse win because csv's cost was the O(n²) `field & ch` concat, not the
  materialization. **json/regex are effectively complete as-is** (json 6 ms,
  regex ~15 ms — regex is structurally heavy by design per the original analysis).
- **B1 benefit is doubtful for the benchmark shape** — the per-call kernel setup
  (vector constant broadcasts) executes every *runtime* iteration because it sits
  in the loop body; a shared out-of-line leaf runs the same setup per call, so it
  does not remove the per-iteration cost. The real fix would be loop-invariant
  code motion (hoist the constant setup out of the loop) or a truly scalar
  (d-register, non-`v`) kernel — both large general changes beyond B1's scope.

### Still TODO (deferred — high-risk native codegen, next session)

- **B** transcendental/float kernels (sin/cos/tan/pow/log/… P2 cluster, sqrt,
  leibniz/nbody/mandelbrot). Intricate NEON dd-Horner; precision-gated
  (`runtime_ulp.py`). B1 (shared out-of-line leaf) is the big structural win but
  risky; B2/B3 have small reach and need the %fN float-native carrier plumbing.
- **I** overflow-check elision (fib 108, thread sum 51). Re-examined this pass:
  - I1/I2 need real range/induction dataflow analysis (elide `adds;b.vc` only
    where provably non-overflowing) — the plan's highest-risk change, touches
    every integer +/-. Note fib stays *fallible* (its sum can overflow), so I2
    frees little there.
  - I3 is **not** "near-zero risk" as first scoped: `Error.source` is observable
    (test framework §22), so an outlined overflow handler must preserve the
    per-site source loc AND the per-TRAP-context routing (`error_exit_destination`
    varies by nesting) — so it can't be one shared stub. Real, delicate work.
- **K2 (NEON popcount)** — DONE. Added `Backend::is_aarch64()`, the `v128.cnt8b` /
  `v128.addv8b` MIR ops (`CNT Vd.8B` + `ADDV Bd, Vn.8B` encodings), and an aarch64
  branch in `lower_bits_popcount` (dup→cnt→addv→umov); x86/riscv keep the exact
  SWAR (`is_aarch64` false, they never see the new ops — zero change, verified by
  0-diff gate). popCount verified exact (0,1,7,255,-1,0x5555,2^62,…). Bits ops
  6.63→~6.55 ms — **neutral** on this benchmark (popCount is 1/18 of the loop and
  the GPR↔SIMD move latency offsets the fewer instructions), but a correct,
  faithful K2 that helps popcount-heavy code. **K1** (keep operands
  register-resident across a fused bits expression) is a general regalloc concern,
  not bits-specific — not separately addressed.
- **B math** the real fix is LICM (hoist the loop-invariant vector-constant setup
  out of the runtime loop) or a scalar-register (non-`v`) kernel rewrite — both
  large *general* changes, each multi-day, not the B1 text as written.
- **E3/string slice** — no defined change beyond "benefits from A's throughput".

All landed changes gated: full acceptance 942, artifact-gate 0-diff
(byte-deterministic), every affected checksum unchanged.

This is the **master plan + Task-1 ordered priority list** for the benchmark
performance push. It classifies every row in the current benchmark logs against
the four goals, orders the work, and indexes the fix sub-plans (Task 2). The
new-benchmark coverage plan (Task 3) is a separate document,
`planning/plan-40-benchmark-coverage.md`.

Source logs (all `--run` matched, one timestamp `20260712-225839`):
`benchmark/mfb-*.log`, `benchmark/c-O0-*.log`, `benchmark/c-O2-*.log`,
`benchmark/python-*.log`. Startup is **excluded** — every workload is wrapped in
`datetime::monotonicNanos()` inside `test_*` (`benchmark/mfb/src/main.mfb:179`
etc.), so the medians are pure workload time. **Re-measure with the same
`--run` count as these logs** when validating a fix (README: median is the metric;
average is dragged by OS-scheduling outliers).

## The goals (priority order)

A benchmark's **priority = the first goal it fails**. Work the lowest-numbered
failures first.

1. **G1** — mfb (MED) **< python** (MED).
2. **G2** — mfb ≤ c-O0 + **10 ms**.
3. **G3** — mfb ≤ c-O0 + **5 ms**.
4. **G4** — mfb ≤ c-O2 + **5 ms**.

**Override:** any mfb MED **≤ 5 ms is already complete**, regardless of G1–G4
(within the margin of error). A benchmark is otherwise **complete only when it
beats all four**. `math fixed` and `vector fixed` have **no cross-language
baseline** (Fixed is mfb-only) and are therefore excluded from G1–G4 scoring; they
are tracked for regressions only.

## Scorecard summary

| Bucket | Count | Meaning |
|--------|------:|---------|
| **P1** (fails G1, loses to Python) | 27 | highest priority |
| **P2** (fails G2, > c-O0 + 10 ms) | 18 | |
| **P3** (fails G3, > c-O0 + 5 ms) | 3 | |
| **P4** (fails G4, > c-O2 + 5 ms) | 3 | lowest priority |
| no-baseline (Fixed, mfb-only) | 2 | excluded from scoring |
| complete (passes all 4, or ≤ 5 ms) | 60 | done |

Total scored rows fail-set = **51** non-complete benchmarks across P1–P4.

---

## Task 1 — ordered priority list

Within each priority band, ordered worst-first by mfb median (biggest absolute
offenders first — they carry the most measurement contamination and user pain).
`Δpy`/`ΔO0`/`ΔO2` are `mfb − baseline` (ms). The **Sub-plan** column maps each row
to the fix that covers it (Task 2).

### P1 — loses to Python (fails G1) — do these first

| # | group/bench | mfb | py | ΔO0 | Sub-plan |
|--:|-------------|----:|---:|----:|----------|
| 1 | list **sortBy** | 647.39 | 3.72 | +645 | **A** collections copy/arena |
| 2 | bignum **modmul** | 228.13 | 189.57 | +223 | **D** bignum limbs |
| 3 | list **window** | 202.80 | 8.63 | +201 | **A** |
| 4 | string **case** | 155.37 | 29.57 | +130 | **E** unicode case |
| 5 | bignum **modexp** | 122.83 | 104.46 | +120 | **D** |
| 6 | liststr **query** | 37.84 | 15.71 | +36 | **A** |
| 7 | list **copy** | 32.20 | 2.35 | +32 | **A** |
| 8 | list **chunks** | 29.38 | 1.70 | +29 | **A** |
| 9 | io **write** | 26.68 | 2.52 | +25 | **F** io buffering |
| 10 | list **reduceRight** | 23.49 | 9.01 | +23 | **A** |
| 11 | parse **csv** | 20.20 | 0.75 | +20 | **G** parse packages |
| 12 | liststr **hof** | 19.70 | 3.02 | +18 | **A** |
| 13 | list **partition** | 18.37 | 7.28 | +18 | **A** |
| 14 | parse **regex** | 15.74 | 0.02 | +16 | **G** (verify parity) |
| 15 | list **findIndex** | 13.83 | 11.39 | +13 | **A** |
| 16 | list **findLastIndex** | 13.81 | 11.38 | +13 | **A** |
| 17 | list **flatten** | 12.50 | 2.79 | +12 | **A** |
| 18 | list **drop** | 10.94 | 0.32 | +11 | **A** |
| 19 | list **take** | 10.47 | 0.36 | +10 | **A** |
| 20 | list **removeAt** | 10.01 | 0.09 | +10 | **A** |
| 21 | list **insert** | 9.80 | 0.21 | +10 | **A** |
| 22 | list **zip** | 7.57 | 1.91 | +7 | **A** |
| 23 | map **str_ops** | 7.57 | 2.79 | +2.8 | **H** map |
| 24 | liststr **build** | 6.93 | 0.14 | +6.9 | **A** |
| 25 | map **int_ops** | 6.49 | 1.82 | +3.6 | **H** |
| 26 | parse **json** | 5.84 | 0.22 | +5.6 | **G** |
| 27 | list **any** | 5.53 | 5.30 | +5.3 | **A** (marginal, +0.23 vs py) |

### P2 — > c-O0 + 10 ms (fails G2)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | recurse **fib** | 108.55 | 47.97 | +60.6 | **I** overflow-check |
| 2 | vector **int** | 99.08 | 6.76 | +92.3 | **C** vector inline |
| 3 | math **simd** | 92.39 | 9.83 | +82.6 | **B** |
| 4 | math **pow** | 90.39 | 18.38 | +72.0 | **B** transcendentals |
| 5 | math **tan** | 72.89 | 9.48 | +63.4 | **B** |
| 6 | thread **sum** | 51.66 | 9.41 | +42.3 | **I** overflow-check |
| 7 | vector **math** | 47.47 | 4.64 | +42.8 | **C** vector inline |
| 8 | string **slice** | 37.98 | 23.55 | +14.4 | **E** |
| 9 | math **log10** | 37.36 | 7.85 | +29.5 | **B** |
| 10 | math **log** | 35.59 | 7.88 | +27.7 | **B** |
| 11 | math **cos** | 32.80 | 7.92 | +24.9 | **B** |
| 12 | math **sin** | 32.67 | 8.01 | +24.7 | **B** |
| 13 | math **atan2** | 31.36 | 14.09 | +17.3 | **B** |
| 14 | math **acos** | 29.70 | 8.85 | +20.9 | **B** |
| 15 | math **asin** | 29.36 | 10.13 | +19.2 | **B** |
| 16 | vector **float** | 27.59 | 6.47 | +21.1 | **C** vector inline |
| 17 | math **atan** | 24.48 | 8.08 | +16.4 | **B** |
| 18 | math **exp** | 19.90 | 7.91 | +12.0 | **B** |

### P3 — > c-O0 + 5 ms (fails G3)

| # | group/bench | mfb | c-O0 | ΔO0 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | float **nbody** | 20.53 | 12.34 | +8.2 | **B** float codegen |
| 2 | float **leibniz** | 10.07 | 3.77 | +6.3 | **B** |
| 3 | list **all** | 5.31 | 0.24 | +5.07 | **A** (marginal, +0.07 over G3) |

### P4 — > c-O2 + 5 ms (fails G4)

| # | group/bench | mfb | c-O2 | ΔO2 | Sub-plan |
|--:|-------------|----:|-----:|----:|----------|
| 1 | float **mandelbrot** | 53.06 | 19.82 | +33.2 | **B** (already beats c-O0) |
| 2 | math **sqrt** | 8.94 | 1.86 | +7.1 | **B** |
| 3 | bits **ops** | 6.63 | 0.63 | +6.0 | **K** bits (small) |

### Excluded / already complete

- **No baseline (Fixed, mfb-only):** `math fixed` (29.37), `vector fixed` (14.57).
  Not scored; regression-track only.
- **Complete (passes all 4, or ≤ 5 ms):** 60 rows incl. `string search`,
  `recurse ackermann`, `math float`/`int`, `list distinct/reduce/filter/…`,
  `map set/lookup`, `parse` checksums, `io read`, `primes`, all trivial list ops.

---

## Task 2 — fix sub-plans (index)

Grouped by **shared root cause** so one fix retires many benchmarks. Ordered by
aggregate priority (how many P1s it clears, biggest offenders first). Each has its
own `plan-39-<letter>-*.md`.

| Sub-plan | Covers (benchmarks) | Priority reach | Root cause (see sub-plan) |
|----------|---------------------|----------------|---------------------------|
| **A** collections source-algo + arena | list sortBy, window, copy, chunks, reduceRight, partition, flatten, findIndex, findLastIndex, take, drop, removeAt, insert, zip, any, all; liststr query/hof/build | 17×P1 + 1×P3 | O(n²)/transform-materialize source funcs + slice sub-alloc churn (arena is secondary) |
| **B** transcendental + float kernels | math sin/cos/tan/atan2/asin/acos/atan/exp/log/log10/pow/sqrt/simd; float leibniz/nbody/mandelbrot | 12×P2 + 2×P3 + 2×P4 | per-call inline kernel setup re-emitted 2M× + dd Horner + finiteness overhead |
| **C** vector inline extension | vector int/math/float | 3×P2 | Integer/Fixed path is scalarized FUNC calls + per-op arena block + software isqrt (no SIMD on either path) |
| **D** bignum limbs | bignum modmul, modexp | 2×P1 | bit-serial O(nbits) bnMod → ~300k fresh limb-list allocs |
| **E** unicode case + slice | string case, string slice | 1×P1 + 1×P2 | per-codepoint 11-deep Unicode-table binary search, no ASCII fast path (CPU-bound, NOT arena) |
| **F** io write buffering | io write | 1×P1 | **benchmark misconfig** — buffering not enabled → 20k syscalls |
| **G** parse packages | parse csv, json, regex | 3×P1 | source packages scan a grapheme-materialized `List OF String` (shares A's churn) |
| **H** map churn | map int_ops, str_ops | 2×P1 | merge/mapValues source generics rebuild intermediate lists |
| **I** overflow-check + call elision | recurse fib, thread sum | 2×P2 | mandatory integer overflow-check `b.vc` + inline error block + post-call result-tag propagation on every op/call |
| **K** bits tightening | bits ops | 1×P4 | operands reloaded from stack + SWAR popcount vs NEON CNT |

> Sub-plan bodies below are grounded in the investigation (root-cause evidence with
> `file:line`). Key findings that reshaped the grouping:
> - **The arena is NOT the primary cause.** The historical quadratic (address-ordered
>   coalescing on large free) was already fixed (plan-25-A); free/alloc are O(1). The
>   collections offenders are **source-package algorithm/constant-factor** problems
>   (O(n²) insertion sort, transform-materialize-then-scan, reverse-copy, per-piece
>   slice sub-allocation). A is mostly `.mfb` source edits, not an allocator rewrite.
>   A residual mixed-size large-bin collision risk (F6) is secondary.
> - **string case and bignum do NOT share a root cause.** case is CPU-bound (Unicode
>   table search); bignum is allocation-churn from a bit-serial algorithm. Separate.
> - **io write is a benchmark misconfiguration**, not runtime slowness (F).
> - **fib and thread sum share ONE root cause** (integer overflow-check + call-tag
>   propagation) → merged into sub-plan I. This also partially lifts vector int (C).
> - **No SIMD exists** on the vector path (ISA `.2d` blocker, plan-01-simd Ph4); float
>   vectors are fast only because their hot members inline register-native.
> Highest leverage: **A** (17 P1s, contained source edits) and **B** (12 P2s).

### Sub-plan B — transcendental + float kernels

**Covers (16):** math sin, cos, tan, atan2, asin, acos, atan, exp, log, log10, pow
(P2), simd (P2, the 2-lane array overload of the same kernels), sqrt (P4); float
leibniz, nbody (P3), mandelbrot (P4).

**Mechanism.** All `Float` math is native NEON `f64` emitted **inline at every call
site** (`src/target/shared/code/builder_math.rs:9` `lower_math_call`; scalar path
`builder_simd_float_math.rs:452` `lower_simd_float_scalar`). Workload is 2M kernel
calls per row (`benchmark/mfb/src/math.mfb:24-30`). No libm.

**Root cause (file:line).**
1. **Per-call loop-invariant setup re-emitted 2M×.** `emit_float_kernel_setup`
   (`builder_simd_float_math.rs:485`) runs every call: `emit_load_math_pool_base`
   (`:1126`, adrp+add) + 6–10 `ldr q` constant broadcasts (`:487-527`) — not
   hoistable because lowering is per-expression. A real libm leaf pays this once.
2. **Double-double compensated Horner** (`emit_compensated_horner:1067`) ≈ 10-12
   vec ops/coeff; **tan worst** — computes sin_r + cos_r as full dd **and** a
   dd-divide (`emit_tan_divide:874`).
3. **Per-call finiteness reduce** on finite inputs: `emit_float_error_reduce:413`
   extracts *both* lanes though scalar uses only lane 0; `emit_result_nan_into_mask:438`.
4. **invtrig runs full 4-segment branchless atan** (4 fdiv) for a single lane —
   sin/cos/tan already branch to one poly, atan/asin/acos/atan2 do not (`:465-469`).
5. **sqrt = 1-instr `fsqrt` drowned in GPR↔FP shuttles** (`builder_math.rs:1145`):
   materialize_float → move back to d → fcmp → fsqrt → move to GPR → caller moves
   to d again. ~3-4 cross-domain moves around a pipelineable fsqrt.
6. **leibniz/nbody:** arithmetic is clean d-native (no per-op check, plan-17), but
   `observe_promoted_float`→`emit_float_result_check_fp` (`builder_math.rs:1284`)
   **re-materializes the +inf immediate every boundary check** (~6 instr × 2M).

**Fixes (semantics-preserving — NO precision-contract change; dd stays).**
- **B1 (largest):** emit each scalar transcendental as **one shared out-of-line
  leaf** (`bl`) instead of inline at 2M sites — setup runs once in callee,
  bit-identical to the array overload. Closes most of the P2 math band.
- **B2 (sqrt, small+safe):** keep `lower_math_sqrt` d-native — `fcmp d,#0`→`b.lt`→
  `fsqrt d`→return FP reg; drop the materialize + move-back. Targets sqrt P4.
- **B3:** hoist the `+inf` constant in `emit_float_result_check_fp` into a pinned
  FP reg per function/loop; drop the lane-1 extract+OR on the scalar reduce; single
  `fcmp`-unordered instead of the 4-op nan mask. Helps leibniz/nbody/all.
- **B4:** give atan/asin/acos/atan2 a branching scalar body (one segment, 1 fdiv),
  mirroring sin/cos/tan. Bit-identical. Targets the invtrig cluster.
- **BLOCKED:** dropping dd→plain-double would ~halve op count but changes last-bit
  results — off the table under constraint 2.
- Order: B2, B3 (safe) → B4 (medium) → B1 (structural, biggest).
- Gate: `tools/math-kernels/runtime_ulp.py` + scalar-vs-array bit identity +
  `scripts/artifact-gate.sh`. Overlaps `planning/plan-25-E-math-kernels.md` (pow
  unroll) — fold that in.

### Sub-plan D — bignum limbs

**Covers (2):** bignum modmul (228 ms), modexp (123 ms). Both **P1 (lose to
Python)** — the only two P1s that are *not* collections-op rows.

**Mechanism.** Pure MFBASIC source (`benchmark/mfb/src/main.mfb:546-772`), base-2²⁸
limbs in `List OF Integer`. `bnMod` (`:686`) is **bit-serial**: `nbits≈532`, each
bit calls `bnShl1`/`bnAdd`/`bnSub` — **each allocates a fresh `List OF Integer`**.
~300k fresh small-list allocs per run.

**Root cause.** Allocation/collection-churn bound (same allocator surface as A),
*plus* an inherently O(nbits) reduction algorithm. `collections::get`/`set`/`append`
themselves are on fast paths; the cost is the sheer count of fresh backing buffers
driven by bit-serial `bnMod`.

**Fixes (source-level, no language change — this is benchmark `.mfb` code).**
- **D1 (biggest):** replace bit-serial `bnMod` with **limb-wise / Barrett /
  Montgomery** reduction so it's O(n) limb-ops, not O(nbits) with per-bit allocs.
  Removes the ~250k allocs at the source.
- **D2:** preallocate capacity in `bnAdd`/`bnSub`/`bnShl1` (`r=[]`+append) so ~19-
  limb lists aren't regrown — benefits from A's arena/capacity work too.
- **Depends on A** for the runtime-side allocation throughput; D1 is independent and
  the dominant lever. Correctness gate: bignum checksum unchanged.

### Sub-plan E — unicode case + slice

**Covers (2):** string case (155 ms, **P1**), string slice (38 ms, **P2**).

**Mechanism.** Inline codegen. `lower_strings_case_map`
(`builder_strings_builtins.rs:430`) runs a **two-pass** (count + write) algorithm;
**every codepoint** (incl. ASCII) does an ~11-deep binary search over the full
Unicode case table via `emit_unicode_u32_mapping_lookup` (`unicode.rs:432`) — **no
ASCII/Latin-1 fast path**. `normalizeNfc` (`:580`) is worse: 2 allocs + per-cp NFD
search + full reorder/compose, **no NFC quick-check**. Workload: 50k iters × ~21-cp
strings × case+fold+NFC (`string.mfb:35-45`) ≈ 370M search iterations. **CPU-bound,
NOT arena-bound** — does *not* share A's root cause. Slice ops are single-alloc,
single-pass; slice's residual is just alloc throughput.

**Fixes.**
- **E1 (largest):** ASCII fast path in `lower_strings_case_map` — if cp < 0x80,
  compute case by range check (±32), skip the binary search and the count pass
  (bytes stay 1-wide). Collapses the 3 case ops to a couple instructions each.
- **E2:** NFC quick-check in `lower_strings_normalize_nfc` — if all cp < 0x300 (or
  NFC_QC=Yes, ccc=0) return a plain copy, skip temp alloc + decompose/compose.
- **E3:** range-bound guard before the table search so digits/space resolve O(1).
- **Slice:** low priority; benefits from A's alloc throughput. Gate: string
  checksums + `scripts/artifact-gate.sh`.

### Sub-plan F — io write buffering

**Covers (1):** io write (26.7 ms, **P1**).

**Root cause — benchmark misconfiguration, not runtime slowness.**
`writeLinesFile` (`benchmark/mfb/src/main.mfb:115-121`) does 20000 `fs::writeAll`
**without** `fs::setBuffered(f, TRUE)`. mfb's per-File write buffer (plan-14-B)
defaults **OFF** (`fs_helpers_io.rs:759`; writeAll branches to the unbuffered direct
syscall loop when the flag is 0, `:978-981`) → **20000 `write()` syscalls**. C
(stdio) and Python buffer by default → a handful of syscalls. The comparison is
apples-to-oranges.

**Fixes.**
- **F1 (the fix):** call `fs::setBuffered(f, TRUE)` in `writeLinesFile` so mfb
  buffers like the C/Python mirrors. Machinery already exists
  (`fs_helpers_io.rs:281+`). This is a harness parity fix — legitimate, changes no
  language behavior. Should collapse to ~1-2 ms.
- **F2 (open decision, product not benchmark):** whether mfb's *default* should be
  buffered is a separate question — do **not** change the default just to move a
  number; if raised, treat as a language-UX decision with its own plan. Secondary:
  `toString(i) & "\n"` allocs a String per line (minor, helped by A).

### Sub-plan G — parse packages (csv/json/regex)

**Covers (3):** parse csv (20.2 ms), json (5.84 ms), regex (15.7 ms) — all **P1**.
Workloads verified **fair** (matching checksums 6003000/5000/200; regex compiles
once per call — *not* a recompile bug; `fs::readText` is outside the timer).

**Mechanism.** All three are **MFBASIC-source packages**
(`src/builtins/{csv,json,regex}_package.mfb`). csv/json first materialize the whole
input into a `List OF String` of one-grapheme strings via `strings::graphemes`
(`csv_package.mfb:22`, `json_package.mfb:293`) — even for pure-ASCII input — then
scan element-by-element with value-semantic `&` concat + `append`.

**Root cause.**
- csv/json: **grapheme materialization** (~24k-element String list for ~24 KB
  ASCII) + per-char `field = field & ch` (`csv_package.mfb:72`, O(n²) growth) +
  nested value-semantic `append`. Feeds A's transient-churn arena path.
- regex: interpreted CPS backtracking matcher — caps `List OF Integer` **copied on
  every capture** (`regex_package.mfb:674`), fresh caps list per `tryAt` (`:799`),
  union/record node allocs per continuation (`:744,761`), restart at every position
  (`:804`). Structurally heavy vs C POSIX NFA / Python C `re`. Fair but ~1000× by
  construction.

**Fixes (runtime + source-package, no language change).**
- **G1:** byte/scalar cursor over the source String instead of `strings::graphemes`
  materialization (regex already has `__regex_toScalars` via one-pass utf32Encode) —
  avoids the 24k single-grapheme list and per-char concats.
- **G2:** csv field builder → chunk-list + `strings::join` at field end (json string
  parser already does this, `json_package.mfb:446-477`) — kills O(n²) growth.
- **G3 (regex):** skip caps threading for zero-capture-group patterns; in-place MUT
  caps buffer within a `tryAt`; literal/first-char pre-filter in `__regex_searchFrom`.
- **Shares A's arena/copy path** — some csv/json gain comes free once A lands.
  Correctness gate: the three parse checksums.

### Sub-plan H — map churn

**Covers (2):** map int_ops (6.49 ms), str_ops (7.57 ms) — both **P1**.

**Mechanism.** `set`/`get` are native (FNV-1a inline probe + `lower_map_set_in_place`
`builder_inplace_assign.rs:300`, O(1) amortized) — these are fine. The 2-3× gap is
`merge`/`mapValues`/`keys`/`values`, which are **MFBASIC-source generics**
(`collections_package.mfb:185,269`) that rebuild fresh Lists/Maps and call the mapper
via indirect FUNC calls; run 50 passes over ~250-entry maps (`map.mfb:64-133`).

**Root cause.** Source-level materialization + indirect-call overhead in the coverage
ops; same value-copy/arena family as A but linear and modest (not catastrophic).

**Fixes.** **H1:** native `merge`/`mapValues` (or at least stop `mapValues` building
three intermediate lists ks/vs/us, `collections_package.mfb:186-188`). **Depends on
A** for the underlying alloc throughput; low structural risk, modest gain. Lower
priority than the P1 collection rows since the delta is small.

### Sub-plan K — bits tightening

**Covers (1):** bits ops (6.63 ms, **P4** — already beats Python by 100×, only
> c-O2 + 5 ms).

**Mechanism.** Every op is inline hardware (`builder_bits.rs:81,159`); `popCount` is
a ~12-instr SWAR (`:220`). Workload 200k iters × ~15 ops (`bits.mfb:20`).

**Root cause.** Operands re-materialized from stack slots each statement (not
register-resident across the fused expression) + SWAR popcount vs a single NEON
`CNT`+`addv`.

**Fixes.** **K1:** keep operands register-resident across the expression; **K2:**
lower `popCount` to NEON `CNT` on a d-reg + horizontal add. Small, optional — lowest
priority in the whole plan (already within 6 ms of c-O2 and crushes Python).

### Sub-plan A — collections source-algorithm + arena (the big one)

**Covers (18):** list sortBy, window, copy, chunks, reduceRight, partition, flatten,
findIndex, findLastIndex, take, drop, removeAt, insert, zip, any (P1), all (P3);
liststr query, hof, build (P1). Retires 17 of the 27 P1 rows.

**Mechanism.** `collections::` splits into **native** members (inline codegen —
get/set/append/prepend/insert/removeAt/filter/reduce/… , `src/builtins/collections.rs:47`)
and **source generics** in `src/builtins/collections_package.mfb` (sort/sortBy/take/
drop/reduceRight/any/all/findIndex/findLastIndex/groupBy/mapValues/flatten/zip/chunks/
window/distinct/merge/partition). The in-place fast paths **do** fire inside the source
generics (append/set stay O(1)), so the pathology is **algorithm + constant factor +
sub-allocation volume**, NOT per-op whole-list copying. Nearly every worst offender is
a source generic.

**Root cause per group (file:line, workload from `benchmark/mfb/src/list.mfb`).**
1. **O(n²) insertion sort — sortBy 647 ms.** `collections_package.mfb:13-27` (sort),
   `:67-85` (sortBy). Benchmark keys are *descending* (`keyNeg` over a monotonic list)
   → insertion sort's **worst case**: 500²/2 ≈ 125k inner iters × 200 calls. Python
   Timsort is O(n log n).
2. **transform-materialize-then-scan — partition 18.4, findIndex/findLastIndex 13.8,
   any 5.5, all 5.3.** `any/all/findIndex/findLastIndex/partition`
   (`collections_package.mfb:105-161,279-294`) each call `collections::transform` first,
   allocating a whole intermediate `List OF Boolean` and running the predicate on
   **every** element, then loop again — defeats short-circuit + extra pass/alloc.
3. **reverse-then-delegate — reduceRight 23.5.** `__collections_reduceRight`
   (`:100-103`) builds a full reversed copy via `__collections_reverse` then calls
   reduce — extra O(n) copy+alloc per call.
4. **per-piece slice sub-allocation — window 202, chunks 29, take 10.5, drop 10.9,
   zip 7.6.** window/chunks (`:224-254`) call `__collections_slice` (`:55-63`) per
   piece, each a fresh element-by-element sub-list. window = **991 sub-allocs/call ×
   100 = ~99k small alloc/free per run**. take/drop/zip rebuild one element at a time
   instead of a bulk range copy.
5. **distinct O(n²)** (`:256-267`) — `contains` membership per element (already
   COMPLETE at 1.9 ms but shares the fix).
6. **copy 32 ms** — value-semantics deep copy (`list.mfb:68-74`), inherent, not a bug.
7. **insert 9.8 / removeAt 10.0** — native ops, but the benchmark does O(n)
   front/middle mutation O(n) times → O(n²) shifts; workload-inherent.

**Arena (secondary).** Already O(1) free (quick bins ≤2048 + 64 hashed large bins,
`entry_and_arena.rs:1621-1655`) and O(1) same-size alloc; the historical coalescing
quadratic is gone (plan-25-A). **Residual:** large-bin alloc matches **exact size
only** (`:888-915`); mixed-size large churn walks collision chains / falls to first-fit
+ drain. Real but secondary — the source fixes capture most of the gap.

**Fixes (all preserve results/errors/value-semantics; mostly `.mfb` source edits).**
- **A1:** merge sort (stable, bottom-up) for `__collections_sort`/`sortBy` — O(n log n),
  same stability + comparison order. sortBy 647 → ~15 ms est. **Biggest single win.**
- **A2:** single-pass inline predicate for any/all/findIndex/findLastIndex/partition —
  drop the `transform` flags list, restore short-circuit (call count only decreases →
  unobservable). Clears 4 P1 + the P3.
- **A3:** backward `reduceRight` fold (no reversed copy).
- **A4:** native contiguous-range slice intrinsic (memcpy-style, like
  `try_inplace_bulk_append` `builder_inplace_assign.rs:99-169`) + output capacity
  reservation; build take/drop/mid/chunks/window/slice/zip on it. Widest reach,
  highest effort. Clears window/chunks/take/drop/zip + liststr reshape.
- **A5:** `distinct` via a `Map` seen-set (O(1) membership), first-occurrence order
  preserved.
- **A6 (arena, optional/secondary):** best-fit large-bin with bounded split, or raise
  `ARENA_LARGE_BIN_COUNT` (64, `error_constants.rs:397`) — closes the mixed-churn tail.
- Order by ROI: A1, A2, A3, A5 (small contained source edits, huge wins) → A4 (native
  slice, broad) → A6 (arena tail). Gate: every list checksum unchanged +
  `scripts/artifact-gate.sh` for the native A4 work. **A also lifts D2/G/H** (shared
  allocation throughput) — sequence A before finalizing those.

### Sub-plan C — vector inline extension

**Covers (3):** vector int (99 ms, worst P2), math (47 ms), float (28 ms).

**Mechanism / root cause (file:line).** `vector::` is a source companion
(`src/builtins/vector_package.mfb`); types are plain records, ops are FUNCs. A
register-native carrier (`builder_vector_inline.rs:21-35`) keeps lanes as scalar
8-byte values — **but there is no NEON `.2d` SIMD on either path** (ISA blocker,
plan-01-simd Ph4). Op-inlining is **Float-only** (`builder_vector_inline.rs:26,65-86`):
only Float scale/dot/lerp/length/distance/cross inline; **every Fixed and Integer op
is an out-of-line FUNC call**. So vector int (99 ms, 14× c-O0) pays: (1) out-of-line
FUNC call per op, (2) **per-op arena block** — passing a register-native vector into a
FUNC materializes a fresh N×8 block (`vector_value_as_block`→`emit_build_inlined_record`,
`:138-172`), ~½M materialize/free per run, (3) **software Newton-loop integer sqrt**
(`__vector_isqrtRound`/`isqrtFloor`, `vector_package.mfb:70-92`) where Float `length`
is one hardware FSQRT. c-O0 is also scalar but pays none of this.

**Fixes (semantics-preserving).**
- **C1:** extend the inline rewrite (`builder_vector_inline.rs:65-86`) to Fixed/Integer
  pure-arithmetic members (dot/scale/length²/distance/lerp/cross) — removes the FUNC
  call **and** the boundary materialization/alloc. Biggest int win.
- **C2:** FSQRT-seeded integer sqrt (double approx + one integer correction, still
  deterministic round-half-away) replacing the unbounded Newton `WHILE`.
- **C3:** keep integer vectors register-native across an inlined op chain so
  `length(scale(a,b))` never materializes an intermediate block.
- Gate: vector checksums + `scripts/artifact-gate.sh`. (vector fixed is mfb-only,
  no goal — but C1/C2 lift it too.)

### Sub-plan I — integer overflow-check + call-tag-propagation elision

**Covers (2):** recurse fib (108 ms, P2), thread sum (51.7 ms, P2). **Shared root
cause**; also partially lifts vector int (C).

**Root cause (file:line).** Every Integer/Fixed `+`/`-` lowers to a set-flags op **plus
an overflow branch** and an **inline never-taken error-return block**
(`emit_integer_binary`, `builder_numeric.rs:871,880-912`; check `:1185`; inline block
`emit_error_code_return`, `builder_codegen_primitives.rs:266,375`). C-O0 emits a bare
`add`/`sub`. On top, every call to a fallible user FUNC emits a **post-call result-tag
check + inline propagation block** (`emit_call`/`builder_emit_helpers.rs:179-189`).
- **fib** (`main.mfb:170`, ~29.9M calls): 3 overflow checks (`n-1`, `n-2`, the sum) +
  2 post-call tag checks per invocation ≈ 5 extra branches. Prologue/epilogue is
  minimal (`codegen_utils.rs:345`) — not the problem.
- **thread sum** (`main.mfb:922`, worker `bench_workers .../lib.mfb:6-15`, 40M-iter
  loop): thread spawn/join/queue is negligible (4 spawns amortized over 40M); the cost
  is `total+i` and `i+1` **both overflow-checked** in the hot loop (2× `adds;b.vc`)
  stretching the accumulator dependency chain.

**Fixes (semantics-preserving — overflow still trapped where it can actually occur).**
- **I1:** range/bounds-based overflow-check elision where operands are provably
  non-overflowing (e.g. `n-1`, `n-2` under the `n≥2` guard; bounded loop induction).
  Skips the `b.vc` + inline block. Same observable behavior (no overflow was possible).
- **I2:** **infallibility analysis** — a FUNC with no `FAIL`, no fallible builtin, and
  (post-I1) no trapping arithmetic is infallible; its callers drop the result-tag check
  + propagation block (`builder_emit_helpers.rs:179-185`). Compounds with I1: eliding
  fib's overflow checks makes it infallible → both caller tag checks vanish.
- **I3:** fully out-of-line the shared `emit_error_*_return` block to one per-error-code
  stub (currently inline at each site, `builder_codegen_primitives.rs:375`) — cuts
  I-cache pressure on hot arithmetic loops even where the check must stay.
- **I4:** tighter register allocation of loop-carried vars; return value directly in the
  consumed register (drop the per-call `mov` out of `RESULT_VALUE_REGISTER`).
- **Risk/scope:** I1/I2 are a real dataflow/effect analysis touching core codegen —
  the highest-risk sub-plan. Gate hard: `scripts/artifact-gate.sh` (4-target byte
  determinism), full `tests/` (overflow-trap tests MUST still pass — elision only where
  provably safe), and re-measure fib/thread. This is also a general language speedup
  beyond the benchmark.

* Overflow elision only with proofs that existing tests encode (no “looks small” heuristics without range analysis).
* Infallibility after I1 must not collapse traps that depend on wrapping-at-bitwidth — only elide where paths are mathematically safe under language Integer semantics.

## Validation Plan (all sub-plans)

- Correctness first: every fix must produce identical observable output
  (benchmark checksums on stderr: `csv=6003000`, `json=5000`, `regex=200`, plus
  each group's printed checksum) and pass `scripts/test-accept.sh` +
  `tests/func_*`. **No language/semantic/syntax change** (constraint 2).
- Re-measure the affected group with the **same `--run`** as the source logs;
  compare medians; confirm the row's priority band improved (ideally to complete).
- Codegen changes: `scripts/artifact-gate.sh` (byte-deterministic 4-target
  self-diff) where applicable; math changes gated by `tools/math-kernels/runtime_ulp.py`.

## Open Decisions

- **A is source edits, not an arena rewrite** — investigation showed the arena is
  already O(1) (the coalescing quadratic was fixed in plan-25-A); the collections
  offenders are source-package algorithm/constant-factor problems. So A's high-ROII
  levers (A1–A3, A5) are contained `.mfb` edits, low risk. The native slice
  intrinsic (A4) and the arena best-fit tail (A6) are the only deeper changes, and
  A6 is optional. Recommended: source fixes first, native slice second, arena tail
  only if the mixed-churn tail still shows.
  Decision: follow recommendation.
- **io write (F) is a benchmark-harness parity fix, not a runtime change** — enable
  `fs::setBuffered` so mfb matches C stdio / Python default buffering. Whether mfb's
  *default* should become buffered is a separate product decision — do **not** flip
  the default just to move a benchmark number.
  Decision: follow recommendation, update test, not the default.
- **Sub-plan I is the risk concentration** — overflow-check elision + infallibility
  analysis touch core codegen and must not weaken the overflow-trap contract. If the
  analysis proves too invasive, I3 (out-of-line error stub) alone still helps hot
  loops at near-zero risk; land it first, gate I1/I2 behind the full trap test suite.
  Decision: follow recommendation, I3 first.
- **`math simd` precision** — the array overload shares B's kernels; B1's shared-leaf
  refactor must keep the scalar-vs-array bit-identity contract. No fast-math variant
  (would add a semantic surface — rejected).
  Decision: Agreed.
- **"complete" cutoffs are marginal for two rows** — `list any` (+0.23 ms vs py) and
  `list all` (+0.07 ms over G3) are inside measurement noise; they fall out of A2 for
  free. Do not over-engineer them.
  Decision: follow recommendation, do not over-engineer them.
- **Fixed rows have no baseline** — `math fixed` (29.4 ms), `vector fixed` (14.6 ms)
  can't be scored against G1–G4. C1/C2 and B improve them incidentally; track for
  regression only, do not gate the plan on them.
  Decision: Agreed.
