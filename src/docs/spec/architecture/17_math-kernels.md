# Math Kernels

Every `Float` and `Fixed` math result MFBASIC produces is computed by a
hand-written **in-tree kernel** — no platform math library is ever linked or
called. The `Float` transcendentals (`exp`, `log`, `log10`, `sin`, `cos`, `tan`,
`atan`, `asin`, `acos`, `atan2`, `pow`) are NEON `f64` kernels; the `Float MOD
Float` operator (`fmod`) is a GPR integer kernel; the `Fixed` transcendentals are
deterministic Q32.32 routines. This topic specifies the accuracy and determinism
contract those kernels meet, how they are validated, and the one place the
behavior deliberately diverges from a system C library.

The lowering dispatch is [[src/target/shared/code/builder_math.rs:lower_math_call]];
the `Float` NEON kernels live in
[[src/target/shared/code/builder_simd_float_math.rs]] (with Remez coefficients in
[[src/target/shared/code/simd_kernel_coeffs.rs]]), `pow` in
[[src/target/shared/code/builder_pow.rs:emit_pow_scalar]], and `fmod` in
[[src/target/shared/code/builder_numeric.rs:emit_float_fmod]]. The scalar and array
overloads of each function **share one kernel**, so `math::f(x)` and
`math::f([x])[0]` are bit-identical (`./mfb spec language builtin-functions`).

## Determinism contract

The kernels contain no platform `branch_link` to a math symbol — verified by the
`no_libm_math_imports` regression test in both
[[src/target/macos_aarch64/plan.rs]] and [[src/target/linux_aarch64/plan.rs]], and
observable as the absence of `_pow`/`_sin`/`_fmod`/… in a built binary's import
table. Because the same hand-written code runs on every target and uses only IEEE
`f64` operations (with FMA where a kernel's reduction calls for it), **a math
result is bit-identical on macOS, Linux-glibc, and Linux-musl** — the same
property `Fixed` already had, now extended to `Float`. There is no
reduction-order ambiguity, no libm version skew, and no last-ULP platform drift.

This extends to the x86_64 and riscv64 backends, and to ordinary user-level
`Float` `a*b±c` expressions, because scalar fused multiply-add is decided in
target-neutral MIR lowering (plan-02) and IEEE-754 FMA is a correctly-rounded,
deterministic operation — so every FMA-capable IEEE target fuses the same
expression to the same bits. Note the distinction in strength: `Fixed`
cross-target bit-identity is a **contractual guarantee** (Q32.32 integer math);
`Float` cross-target bit-identity holds **in practice** under this uniform-fusion
policy but is not contractually guaranteed — the headroom is reserved for a future
target that lacks hardware FMA (`./mfb spec language builtin-functions`).

(`math::rand`/`math::seed` import `getentropy` from libc for their startup seed;
that is the RNG, not math, and it is the only `math::` member that imports
anything.)

## Accuracy

The accuracy bar is **macOS libm**, captured once as bit-pattern reference
vectors in `tests/_data/math_kernel_ref/<fn>.ref`. The kernels meet it as follows:

| Function(s) | Bound | Notes |
|---|---|---|
| `exp`, `log`, `log10`, `sin`, `cos`, `atan`, `asin`, `acos`, `atan2` | **≤1 ULP** of macOS libm | double-double-compensated Remez polynomials; fdlibm 4-segment `atan`; `acos` via the half-angle identity `2·atan(√((1−x)/(1+x)))` |
| `pow` | **≤1 ULP** of macOS libm | fdlibm `__ieee754_pow` in log2 space; negative base with an integer exponent matches libm (`(-2)^3 = -8`) |
| `tan` | **faithfully rounded — ≤1 ULP of the TRUE value** | more accurate than macOS libm; see below |
| `fmod` | **0 ULP — bit-identical** to libm | the IEEE remainder is exactly representable |
| `Fixed` transcendentals, `Fixed MOD` | deterministic Q32.32 | platform-independent by construction; not an `f64` bound |

`acos` deliberately uses the half-angle identity rather than `π/2 − asin(x)`: the
latter cancels catastrophically as `x → +1` (where `acos → 0`), while `1±x` is
exact for `|x| ≤ 1` (Sterbenz), so `2·atan(√((1−x)/(1+x)))` stays ≤1 ULP across the
whole domain. The endpoints fall out of IEEE arithmetic — at `x = −1` the divide
yields `+inf`, `atan(+inf) = π/2`, and `2·(π/2) = π` exactly.

## The oracle is not correctly-rounded — the `tan` deviation

macOS libm is the *bar*, not the *truth*: the captured `.ref` vectors are whatever
macOS computed, and macOS libm is **not** correctly-rounded for every function. In
particular, **macOS `tan` is itself more than 1 ULP off the true value on ~19 of
the `tan.ref` vectors.** MFBASIC's `tan` kernel is faithfully rounded (≤1 ULP of
the *true* value on every primary-domain vector), so at exactly those inputs it
*disagrees with macOS by ~2 ULP while being the more accurate result*.

Two consequences a maintainer should expect:

- The correctness gate for `tan` is **ULP-vs-truth (mpmath), not ULP-vs-macOS**. A
  "miss" against `tan.ref` at one of those ~19 points is macOS being wrong, not the
  kernel.
- A `tan` result can differ in the last bit or two from the host C library's
  `tan` — that is intended and more correct, not a regression.

For every other function the kernel and macOS libm agree to ≤1 ULP, so the
distinction only matters for `tan`.

`pow` is a separate cautionary tale: the natural-log identity `exp(y·log x)` is
**not** faithfully roundable across `pow`'s dynamic range — the `n·ln2` reduction
loses bits and reaches ~10⁹ ULP at `pow(10, 300)`. The kernel instead works in
log2 space with the integer part of `y·log₂x` split off exactly (fdlibm), which is
why the implementation looks nothing like "exp of y times log".

## Validation

Three layers, all offline/in-tree (no network, no Mac required after capture):

1. **Reference vectors** — `tools/math-kernels/capture_ref.c` links macOS libm and
   emits `<fn>.ref` bit-pattern vectors; the committed copies under
   `tests/_data/math_kernel_ref/` *are* macOS libm.
2. **Coefficient/algorithm proof** — `tools/math-kernels/gen_coeffs.py verify`
   reconstructs each kernel in `f64` (mirroring the codegen's reduction + FMA
   sequence) and reports its ULP histogram against the vectors, proving a
   coefficient set meets the bar before any codegen.
3. **Emitted-code proof** — `tools/math-kernels/runtime_ulp.py` drives the
   **actually emitted machine code** over the vectors (recovering each result
   bit-exactly via `toString(x, N)`'s exact decimal expansion) and reports ULP
   against both macOS and the mpmath truth. This is the gate that catches a
   codegen transcription bug a reconstruction would miss, and the one that
   measures the `tan` truth-vs-macOS gap.

See `tools/math-kernels/README.md` for the full tooling, and
`./mfb man math` for the per-function user documentation.

## See Also

* ./mfb spec language builtin-functions — the user-facing `math::` member list and accuracy prose
* ./mfb spec linker import-selection — why no build links `libm`
* ./mfb spec architecture aarch64-instruction-set — the NEON `CodeOp`s the kernels emit
* ./mfb spec diagnostics error-codes — `ErrFloatDomain`/`ErrFloatNan`/`ErrFloatInf`/`ErrInvalidArgument`
