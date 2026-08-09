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
- [ ] **L1 (M1, the only bounded lever)** — coalesce sibling finiteness checks at a shared boundary:
  nbody/mandelbrot's `nzr`/`nzi` are two producers per iteration; a combined `fmax(|nzr|,|nzi|)` vs +Inf
  halves the branch count (bit-identical trap set; keep the earliest `line:char` stamp). Tens of percent on
  nbody/leibniz, not a multiple. Only land if cheap/non-gating.
- [~] **BLOCKED (track for regression):** the transcendental band, fib/thread, io format, and crypto sha256
  cannot reach their bands without breaking the dd-precision / overflow-trap contract, replacing the float
  formatter, or swapping the software crypto core for a C backend. Documented ceilings.

## Acceptance
`tools/math-kernels/runtime_ulp.py` + scalar-vs-array bit identity + all math/float/crypto checksums unchanged.
