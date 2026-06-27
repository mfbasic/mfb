# math-kernels — offline tooling for the NEON f64 transcendental kernels

Supporting tooling for **plan-01-simd Phase 5** (`planning/plan-01-simd.md`
§4.6, Validation Plan, Open Decision #7): the hand-written NEON `f64` polynomial
kernels for the 11 transcendentals (`exp, log, log10, sin, cos, tan, asin, acos,
atan, atan2, pow`) must be validated **≤1 ULP against macOS-libm reference
vectors**, which requires (a) capturing that macOS-libm reference and (b)
generating the minimax polynomial coefficients offline. Neither existed in the
tree; this directory provides both.

This is **build-time/offline tooling only** — nothing here is linked into the
compiler or the runtime. It produces two committed artifacts that the codegen and
its tests consume:

1. **macOS-libm reference vectors** — the accuracy oracle.
2. **Minimax coefficient constants** (Rust) — the kernel polynomials.

The macOS-libm oracle is authoritative by construction: `capture_ref.c` links the
system math library and calls `sin`/`cos`/`exp`/… directly, with **zero
reimplementation**. The captured `(input, expected_bits)` files *are* macOS libm,
so Linux/CI validate against them without needing a Mac.

## Pieces

| File | Role |
|---|---|
| `capture_ref.c` | macOS-libm oracle. Pure filter: reads input bit-patterns, applies the named libm function, writes `input[ y] result` bits. The one place "the macOS-libm value" is defined. **Build/run on the reference Mac.** |
| `gen_inputs.py` | Deterministic representative + boundary input sets per function (domain sweep, breakpoints, range-reduction stress, near-1 walks, seeded random fill). |
| `capture.sh` | Drives `gen_inputs.py → capture_ref` into one `reference/<fn>.ref` per function, each with a provenance header (OS / libm / cc / date). Refuses to run off Darwin. |
| `gen_coeffs.py` | Remez-exchange minimax coefficient generator (arbitrary precision via `mpmath`). Emits named `f64` Rust constants, and `verify`s reconstructed kernels against the captured macOS-libm vectors. |
| `ulp.py` | Stdlib-only IEEE-754 helpers (`ulp_diff`, bit conversions) shared by the above. |
| `requirements.txt` | `mpmath` (pure Python) — the only dependency, and only for `gen_coeffs.py`. |

## Setup

```sh
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt           # mpmath; only needed for gen_coeffs.py
```

`capture_ref.c` / `capture.sh` / `gen_inputs.py` need only a C compiler + python3
(stdlib). The `mpmath` dependency is isolated to `gen_coeffs.py`.

## 1. Capture the macOS-libm reference (run once, on the reference Mac)

```sh
# Writes reference/<fn>.ref (tool-local scratch). For the committed copy, point
# it at the in-tree test-data path the kernel tests read:
./capture.sh ../../tests/_data/math_kernel_ref
```

Each `.ref` file:

```
# GENERATED — macOS libm reference vectors for math::exp …
#   OS: macOS 15.7.7 (24G720)   Kernel: Darwin 24.6.0 arm64   …
# Format: <x_bits> <expected_bits>  (lowercase IEEE-754 hex)
3ff0000000000000 4005bf0a8b145769      # exp(1.0)
…
```

Bit-patterns (not decimal) so the reference is exact and trivially parsed in
Rust. Re-capture only when intentionally re-pinning the oracle; commit the result
and record the OS/libm version (the header does this automatically).

## 2. Generate the coefficients

```sh
python3 gen_coeffs.py gen --out ../../src/<wherever>/kernel_coeffs.rs
```

Five **primitive** reduced approximations are minimax-fitted (`exp, log, sin,
cos, atan`); the other six surface functions are *built from* them per §4.6
(`tan = sin/cos`, `log10 = log·log10(e)`, `asin/acos/atan2` via `atan`,
`pow = exp(y·log x)`) and need no separate polynomial. Output is a Rust file of
`pub const <FN>_COEFFS: [f64; N]` arrays, each documented with the function
approximated, the reduction it assumes, the fit interval/degree, and the achieved
minimax error:

```rust
/// exp: minimax of `exp(x) = 2**n * P(r)`
/// reduction: x = n*ln2 + r,  n = round(x/ln2),  r in [-ln2/2, ln2/2]
/// fit var `r` on [-0.3465…, 0.3465…], degree 11 (relative error)
/// achieved minimax relative error: 3.055e-18 (~0.0138 ULP of the reduced value)
pub const EXP_COEFFS: [f64; 12] = [ 1.0, 1.0, 0.5000000000000018, … ];
```

The polynomial is fitted against the **mathematically exact** function (mpmath at
80 digits), not against libm — fitting against libm would bake libm's own ≈0.5
ULP error into the kernel. macOS libm is the *acceptance* bar (step 3), not the
fit target.

To add/retune a primitive, edit its entry in `CONFIGS` (interval, degree,
relative/absolute) — the Remez engine is general.

## 3. Verify against the macOS-libm oracle

```sh
./capture.sh                       # produce reference/*.ref locally (macOS)
python3 gen_coeffs.py verify       # or: verify exp sin …   (subset)
```

`verify` reconstructs the **full** f64 kernel from the generated coefficients
(the same range-reduction + identity sequence codegen will emit, using hardware
FMA via `math.fma`) and reports its ULP distance from the committed macOS-libm
vectors. This proves a coefficient set meets the ≤1 ULP target **before** any
codegen.

```
fn       primary   <=1ULP  maxULP    extended   <=1ULP     maxULP
exp         1233  100.00%       1           -        -          -
cos         1059  100.00%       1          20    0.00%   8.6e14
…
```

### verify scope (important)

`verify`'s reconstructions are **reference models** that exist to validate the
*coefficients*, not finished kernels. The report buckets every vector:

- **primary** — inputs the reference reduction models faithfully. The `<=1 ULP`
  rate and `maxULP` here measure **coefficient + identity** quality. `exp` and
  `cos` reach 100% / 1 ULP, demonstrating the pipeline reaches the bar
  end-to-end.
- **extended** — large-argument trig vectors (|x| ≳ 2²⁰·π/2) that require a
  **Payne-Hanek** reduction the reference model deliberately omits. Misses here
  measure the *codegen's* large-argument reduction, **not** a coefficient defect.

Two classes of residual gap are, by design, the codegen implementer's Phase-5
work rather than this tool's:

- **Payne-Hanek** large-argument trig reduction (the `extended` bucket).
- **Production identities / extra precision** for the derived functions — most
  notably `pow`, which needs `y·log(x)` evaluated in double-double, and the
  last-ULP argument-segmenting in `atan`/`asin`/`acos`/`log10`. The naive
  reference reconstructions land a few ULP out; the committed coefficients are
  not the limiting factor.

In short: **this tool proves the coefficients and supplies the oracle**; closing
the last ULP on the full kernels (Payne-Hanek, double-double `pow`, segmented
reductions) happens in codegen, measured by re-running `verify` and by the
in-tree Rust kernel tests against the same `.ref` files.

## Regeneration checklist

- Re-pin the oracle (new macOS / libm): re-run `./capture.sh <committed-path>` on
  the reference Mac; commit; the header records the new version.
- Retune a polynomial: edit `CONFIGS` in `gen_coeffs.py`, re-run `gen`, re-run
  `verify`, regenerate the affected Rust constants and goldens.
- Determinism: `gen_inputs.py` and the Remez fit are fully deterministic (seeded
  PRNG, fixed precision) — same inputs in, same vectors/coefficients out.
