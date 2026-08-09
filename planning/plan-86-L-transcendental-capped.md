# plan-86-L — transcendental / float / overflow / formatter kernels (capped)

Sub-plan **L** of [plan-86](plan-86-benchmark-perf.md). **Structurally capped — only L1 is a live lever;
the rest is track-for-regression.**

**Covers (2 P1 + 14 P2 + 4 P3 + 3 P4):** math sin/cos/tan/exp/log/log10/pow/simd/asin/acos/atan/atan2, sqrt;
float leibniz/nbody/mandelbrot; recurse fib; thread sum; io format; crypto sha256.

## Root cause / ceiling
- **Transcendentals + sqrt:** every `Float` math op open-codes a double-double compensated Horner to meet the
  ≤1-ULP / deterministic / no-libm contract (`builder_simd_float_math.rs`, `builder_pow.rs`); `sqrt` is a
  single hardware `float_sqrt_d` (IEEE-exact, optimal). The 3–10× gaps are the structural dd-vs-libm delta;
  achievable semantics-preserving gain ≈ 0.
- **fib/thread:** mandatory checked add under integer-overflow-trap semantics (`emit_integer_binary_checked`);
  the check is non-elidable.
- **io format:** the concat chain is cheap; the cost is the intrinsic `float_format.rs` per-value formatter
  (plan-64 L1 concat-fusion was measured as noise). Formatter-capped.
- **crypto sha256:** software `bits` core vs Python hashlib's C backend — structural.

## Fixes
- [x] ~~**L1 (M1, the only bounded lever)** — coalesce sibling finiteness checks at a shared boundary:
  nbody/mandelbrot's `nzr`/`nzi` are two producers per iteration; a combined `fmax(|nzr|,|nzi|)` vs +Inf
  halves the branch count (bit-identical trap set; keep the earliest `line:char` stamp). Tens of percent on
  nbody/leibniz, not a multiple. Only land if cheap/non-gating.~~ — **moot: fails its OWN "only land if
  cheap/non-gating" condition, and two of its premises are false (evidence below).** Examined the code
  concretely this session:
  1. **The nbody target does NOT exist.** `test_nbody` (`main.mfb:243`) uses fully UNROLLED scalar bodies
     (`x0..x4`, `y0..y4`, `vx0..`, the `e = e - m0*m1/energyDist(...)` chain) — there is no `nzr`/`nzi` sibling
     pair to coalesce. Only `test_mandelbrot` (`main.mfb:485-486`) has it (`LET nzr = zr*zr - zi*zi + re; LET
     nzi = 2*zr*zi + im`). leibniz (`:216`) is a single scalar accumulator, also no pair.
  2. **The "`fmax(|nzr|,|nzi|)` vs +Inf" design is UNSOUND as stated.** `emit_float_result_check`
     (`builder_math.rs:1179`) is a **3-way** predicate: finite→ok, ±Inf→`emit_float_overflow_return` (code
     7-705-00xx overflow), NaN→`emit_float_nan_return` (a DISTINCT error). IEEE `fmax(NaN, x) = x` returns the
     non-NaN operand, so a combined `fmax` check **silently drops** a NaN in `nzr` (a missed trap), and one
     combined compare **cannot distinguish** overflow-vs-NaN nor WHICH value/line errored. A correct coalescing
     must keep BOTH individual checks (for the exact error + `line:char` on the rare non-finite path) and add a
     fast-path "both finite?" combined check that jumps past them — a genuine observation-emission restructure
     (`observe_float` fires eagerly per `LET`), correctness-sensitive, for a **bounded ~5% inner-loop** win on
     mandelbrot (P4 — already **beats c-O0**; c-O2 wins by vectorizing, which this does not touch). That is the
     opposite of "cheap/non-gating," so the box's own gating condition says DO NOT land it. Recorded per
     "correct a false claim / apply the phase's acceptance criterion," not skipped on difficulty.
- [~] **BLOCKED (track for regression):** the transcendental band, fib/thread, io format, and crypto sha256
  cannot reach their bands without breaking the dd-precision / overflow-trap contract, replacing the float
  formatter, or swapping the software crypto core for a C backend. Documented ceilings.

## Acceptance
`tools/math-kernels/runtime_ulp.py` + scalar-vs-array bit identity + all math/float/crypto checksums unchanged.
