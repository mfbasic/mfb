# MFBASIC libm Severance — Hand-Written `pow` / `atan2` / `tan` / `fmod` Plan

Last updated: 2026-06-27

plan-01-simd re-pointed eight Float transcendentals (`exp`, `log`, `log10`,
`sin`, `cos`, `atan`, `asin`, `acos`) off the platform math library onto
hand-written NEON `f64` kernels, validated ≤1 ULP against macOS libm. Four
math surfaces still call out to the platform routine: scalar `math::pow`,
scalar `math::atan2`, scalar `math::tan`, and the `Float MOD Float` operator
(`fmod`). This plan moves those four onto hand-written kernels held to the same
**≤1 ULP-against-macOS-libm** bar (and bit-exact for `fmod`), so that after the
final phase **no MFBASIC program imports a single libm/libSystem math symbol** —
the Linux executable stops importing `libm.so` entirely, and macOS imports zero
`_pow`/`_tan`/`_atan2`/`_fmod` from `libSystem`.

The single behavioral outcome a correct implementation produces: every
`math::pow`/`atan2`/`tan` scalar Float call and every `Float MOD Float` lowers to
in-tree code with no `branch_link` into a platform math symbol, the result is
within ≤1 ULP of macOS libm (0 ULP for `fmod`), `math::f(x)` stays bit-identical
to `math::f([x])[0]`, and results are identical on macOS / Linux-glibc /
Linux-musl.

It complements:

- `./mfb spec language builtin-functions` (§18.2 `math::` member list and the
  "no external math library" / ≤1 ULP claims for the Float kernels;
  `src/spec/language/18_builtin-functions.md`)
- `./mfb spec linker import-selection` and `./mfb spec linker linux-aarch64`
  (the `libm` import row this plan deletes; `src/spec/linker/03_*`, `07_*`)
- `./mfb spec architecture native-ir` (the `link`/`native_call` math symbol
  surface; `src/spec/architecture/13_native-ir.md`)
- `./mfb spec diagnostics error-codes` (`ErrFloatNan` / `ErrFloatInf` /
  `ErrFloatDomain` / `ErrInvalidArgument` raised by the kernels;
  `src/spec/diagnostics/02_error-codes.md`)
- `tools/math-kernels/README.md` + `gen_coeffs.py` (the Remez generator and the
  `verify` harness that scores reconstructions against the committed macOS-libm
  reference vectors in `tests/_data/math_kernel_ref/`)

## 1. Goal

Remove the last four platform-math callers, each held to the ≤1 ULP bar:

| Surface | Today | Target |
|---|---|---|
| `math::pow(Float, Float)` | libm `pow` (`lower_external_math`) | hand-written NEON kernel, ≤1 ULP, incl. negative base + integer exponent (§4.4) |
| `math::atan2(Float, Float)` | libm `atan2` (`lower_external_math`) | re-point onto the **existing** strict-≤1-ULP array `atan2` kernel (§4.2) |
| `math::tan(Float)` | libm `tan` (`lower_external_math`) | upgrade the array `tan` kernel to strict ≤1 ULP, re-point scalar (§4.3) |
| `Float MOD Float` | libm `fmod` (`builder_numeric.rs`) | exact hand-written `fmod` kernel, bit-identical to libm (§4.1) |

Concrete checkable outcomes:

1. A release build of a program using all four surfaces imports **zero** libm /
   libSystem math symbols (`_pow`, `_tan`, `_atan2`, `_fmod`, and on Linux no
   `libm.so` `DT_NEEDED`). Verified by inspecting the emitted import table.
2. Each kernel scores **100% ≤1 ULP** on the primary domain of its
   `tests/_data/math_kernel_ref/<fn>.ref` vectors (`fmod`: 0 ULP / bit-exact),
   measured by an in-tree runtime accuracy test (§Validation).
3. `math::f(x) == math::f([x])[0]` holds bit-for-bit for `pow`/`atan2`/`tan`
   (scalar shares the array kernel), and all results are identical across
   macOS / Linux-glibc / Linux-musl.
4. Existing `func_math_{pow,atan2,tan}_*` and `arithmetic-*-mod-*` tests pass
   unchanged in their **documented** behavior (see the pow negative-base
   decision, §Open Decisions), plus the new full-overload coverage below.
5. `scripts/test-accept.sh` passes.

### Non-goals (explicit constraints)

- **No `Fixed` change.** `pow`/`atan2`/`tan` Fixed overloads and `Fixed MOD`
  already use deterministic Q32.32 paths (`builder_fixed_math.rs`,
  `lower_fixed_external_math`) and never touched libm — they are untouched.
- **No language-surface change.** No new/removed overloads, names, signatures,
  or default-argument behavior. `mfb spec language` member list is unchanged
  except the accuracy prose.
- **No error-code or value/copy/transfer-semantics change.** The kernels raise
  the *same* float error codes the scalar man pages already document; List
  layout, copy, freeze, and thread-transfer rules are untouched.
- **No large-argument trig reduction beyond the shipped envelope.** `tan` uses
  the same medium-range Cody-Waite reduction as the shipped `sin`/`cos`
  (accurate for `|x| < 2^20 · π/2`); Payne-Hanek for huge arguments is out of
  scope here exactly as it is for `sin`/`cos` today. The ≤1 ULP bar is the
  primary domain of the reference vectors, identical to plan-01-simd.
- **`snprintf`/`getentropy`/`pthread_*` stay.** Those are libc, not libm; this
  plan does not touch them.

## 2. Current State

Scalar `math::` lowering dispatches in `builder_math.rs::lower_math` (lines
40–71). After plan-01-simd:

- `exp`/`log`/`log10`/`sin`/`cos`/`atan`/`asin`/`acos` →
  `lower_math_scalar_transcendental` → `lower_simd_float_scalar` (broadcast the
  scalar into both `.2d` lanes, run the array kernel, extract lane 0;
  `builder_simd_float_math.rs:258`). No libm.
- `pow` (`:53`), `atan2` (`:54`), and `tan` (`:67`) still go to
  `lower_external_math` (`:848`), which emits a `branch_link` to the symbol from
  `external_math_symbol` (`:1040`) plus an `external`-binding relocation into the
  platform import. The Fixed path branches to `lower_fixed_external_math` first.
- `Float MOD Float` lowers in `builder_numeric.rs:869` via `external_math_symbol("fmod", …)`
  → libm `fmod`. The import is pulled whenever a `MOD` node exists
  (`symbols.rs:300`, `collect_platform_imports_from_value`).

Import tables that name these symbols:

- Linux: `linux_aarch64/plan.rs:369–380` (`native_call_imports`, maps
  `math.pow`→`pow`, …, `math.fmod`→`fmod`, library `libm.so.6`/`libm.so.1`).
- macOS: `macos_aarch64/plan.rs:483–490` (`_pow`/`_exp`/…/`_fmod` from
  `libSystem`).

These tables still *declare* the full classic set (`sin`/`cos`/`exp`/…), but the
re-pointed eight no longer emit a `native_call`, so those rows are already dead —
only `pow`/`atan2`/`tan`/`fmod` are live importers today.

**The existing kernels we build on** (`builder_simd_float_math.rs`):

- `emit_atan_core` (`:392`) — fdlibm 4-segment `atan`, **already strict ≤1 ULP**.
- `FloatBinaryKernel::Atan2` body (`:856`) — `atan2(y,x)=atan(y/x)+quadrant`,
  built on `emit_atan_core`; **already strict ≤1 ULP** (the array `atan2` ships
  as strict). Scalar `atan2` simply does not route here yet.
- `emit_tan_body` (`:540`) — `tan=sin_r/cos_r` with compensated (double-double)
  `sin_r`/`cos_r` but a plain final `fdiv`. ~99.8% ≤1 ULP, **max 2 ULP near the
  asymptotes** (the README and the `:538` comment flag this as the gap).
- `FloatBinaryKernel::Pow` body (`:868`) — `exp(y·log x)` with double-double
  `y·log(x)`. Faithfully rounded (≤1 ULP for almost all inputs) but **not strict
  on the hard cases**, and it raises `ErrFloatNan` for **any** non-positive base
  (the `log_body` domain mask), so it does **not** compute `(-2)^3 = -8` the way
  libm `pow` does (a behavioral gap, not just accuracy — see §4.4 / Open
  Decisions).

**Validation infrastructure already present:** `tools/math-kernels/` holds
`capture_ref.c` (links macOS libm, emits `<fn>.ref` bit-pattern vectors),
`gen_coeffs.py verify` (reconstructs each kernel in `f64` with `math.fma` and
reports a per-bucket ULP histogram vs the committed vectors), and the committed
vectors in `tests/_data/math_kernel_ref/{tan,pow,atan2,…}.ref`. The `verify`
reconstructions for `tan`/`pow` are the *naive* versions (`ktan = ksin/kcos`,
`kpow = kexp(y·klog x)`) and currently land a few ULP out — this plan upgrades
both the reconstruction and the codegen together.

## 3. Design Overview

The four targets sit at very different distances from done, so they are ordered
by risk, lowest first:

1. **`atan2` scalar — pure plumbing, zero new math.** The array kernel is
   already strict. We only need a scalar entry point: broadcast both operands
   into `.2d`, run `emit_float_binary_body`, extract lane 0 — the binary analog
   of `lower_simd_float_scalar`. This also establishes the scalar-binary seam
   that `pow` reuses.
2. **`fmod` — a new but *exact* kernel.** `fmod` is bit-exact (it returns an
   exactly representable remainder), so "≤1 ULP" is trivially met at 0 ULP. It
   is a GPR integer algorithm (fdlibm `__ieee754_fmod`: align exponents, repeated
   subtractive reduction), not a polynomial — no coefficients, no Remez. Scalar
   only (`MOD` has no array form).
3. **`tan` strict — upgrade one kernel body.** Replace the plain `sin_r/cos_r`
   divide with a strict reconstruction (recommended: fdlibm `__kernel_tan`
   polynomial for the reduced argument, with the cofunction-branch reciprocal),
   so the residual near the poles closes to ≤1 ULP. Then re-point scalar `tan`
   onto the array `Tan` kernel.
4. **`pow` strict — the hard one.** Upgrade the `Pow` body to fdlibm
   `__ieee754_pow`'s reconstruction: a higher-precision `log2(x)`/`2^(y·log2 x)`
   split *and* the sign / integer-exponent / special-case logic so negative
   bases with integer exponents (and `0`/`1`/`inf`/`nan` edge cases) match libm.
   Then re-point scalar `pow`.

The correctness risk concentrates almost entirely in Phases 3 (`tan`) and 4
(`pow`); Phases 1–2 are mechanical and exact. The final phase severs the libm
import declarations and updates the specs/man pages once nothing routes there.

The accuracy methodology is unchanged from plan-01-simd: fit/derive against
mathematically-exact `mpmath`, **verify against macOS-libm reference vectors**.
Any new minimax polynomial (only `tan` may need one, for `__kernel_tan`) is
generated by `gen_coeffs.py` with full provenance; `pow` and `atan2` are
*reconstructions* of existing primitives and need no new polynomial.

## 4. Detailed Design

### 4.1 `fmod` (Float MOD) — exact GPR kernel

`fmod(a, b)` returns `a - n·b` where `n = trunc(a/b)`, computed **exactly** (no
rounding) by the IEEE bitwise algorithm. Implement `__ieee754_fmod` over GPRs:

- Decompose `a`, `b` into sign/exponent/mantissa from their bit patterns.
- Handle the libm special cases up front, matching macOS: `b == 0` or `a` is
  inf/nan → result NaN; `|a| < |b|` → result `a`; `|a| == |b|` → result
  `±0` with `a`'s sign.
- Otherwise align the exponents and perform the subtract-and-shift remainder
  loop (integer mantissa arithmetic), then renormalize and reapply `a`'s sign.

Because the result is exact, the bar is **0 ULP / bit-identical to libm** — no
reference-vector tolerance needed, but we still validate against
`tests/_data/math_kernel_ref/fmod.ref` (new capture) for confidence and to lock
the special cases.

**Zero divisor is already handled and never reaches the kernel.**
`builder_numeric.rs` guards `Float MOD Float` *before* the call:
`float_compare_zero_d("d1")` → `emit_float_domain_return()` raises
`ErrFloatDomain` when the divisor is `0`, then branches to the call only on a
non-zero divisor (`builder_numeric.rs:863–875`). MFBASIC has **no NaN** as a
value, so libm's "return NaN on `MOD 0`" path is unreachable today and stays so —
the existing `ErrFloatDomain` pre-check is preserved verbatim and the new kernel
is only ever entered with a non-zero `b`. The `emit_float_result_check(…,
Overflow)` after the call (`:893`) likewise stays. (Decision: keep the existing
`ErrFloatDomain` guard; the kernel needs no zero-divisor branch.)

Wiring: add `emit_float_fmod(a_loc, b_loc) -> result_loc` (scalar, GPR-only) and
call it from `builder_numeric.rs` in place of the
`external_math_symbol("fmod", …)` / `branch_link` block — leaving the surrounding
zero-check and result-check untouched. Remove the `math.fmod` import collection
from `symbols.rs:300` (and the table rows in Phase 5).

### 4.2 `atan2` scalar — re-point onto the strict array kernel

Add `lower_simd_float_binary_scalar(kernel, left_loc, right_loc, text)` — the
binary analog of `lower_simd_float_scalar`:

```
broadcast left  -> v0 (both lanes)
broadcast right -> v1 (both lanes)
emit_float_binary_setup(kernel)
emit_float_binary_body(kernel)        // existing Atan2 body, strict ≤1 ULP
reduce v22 (ErrFloatNan) -> raise if set
extract lane 0
```

Dispatch: in `lower_math`, route scalar `atan2` (`args.len()==2`, Float) to a new
`lower_math_scalar_binary(FloatBinaryKernel::Atan2, …)` instead of
`lower_external_math`. Fixed `atan2` continues to `lower_fixed_external_math`.
No new math — the array `atan2` already validates strict in the committed
`atan2.ref`.

### 4.3 `tan` strict ≤1 ULP

The gap is the final divide: `sin_r` and `cos_r` are computed in double-double,
but `tan = sin_r / cos_r` is a single `fdiv`, and near an asymptote `cos_r → 0`
amplifies the rounding of the truncated quotient past 1 ULP.

Recommended fix — **fdlibm `__kernel_tan`**: after the shared Cody-Waite
reduction (`emit_sincos_reduce`, reused unchanged), evaluate `tan(r)` directly
from a single odd minimax polynomial in `r` (generated by `gen_coeffs.py` as a
new `TAN_COEFFS` primitive, fitted on `|r| ≤ π/4`), with the fdlibm high/low
split for the leading `r` term. For the quadrant-odd branch the result is the
negative reciprocal `-cos/sin` form; fdlibm computes that reciprocal with a
one-step correction (`-1/(t + small)`) that holds ≤1 ULP. This removes the
catastrophic divide and is the proven strict path.

Alternative — **double-double divide**: keep `sin_r`/`cos_r`, but carry the
quotient as a Newton/double-double reciprocal (`q = sin·(1/cos)` refined once).
Cheaper to implement (no new polynomial) but historically struggles to reach
*strict* ≤1 ULP at the worst near-pole inputs; recommend `__kernel_tan` and
falling back to dd-divide only if the polynomial path proves heavier than
budget. (Open Decision.)

Update `gen_coeffs.py`'s `ktan` reconstruction to mirror whichever path is
chosen so `verify` scores the real kernel, not `ksin/kcos`. Then re-point scalar
`tan`: add `"tan" => FloatKernel::Tan` to `lower_math_scalar_transcendental`'s
match and route scalar `tan` there (drop the `:67` `lower_external_math` arm).

### 4.4 `pow` strict ≤1 ULP, incl. negative base + integer exponent

Two problems with today's `exp(y·log x)`:

1. **Accuracy:** `y·log(x)` in double-double is faithfully rounded but the
   compounded exp/log rounding misses strict ≤1 ULP on pow's hard cases.
2. **Behavior:** `log(x)` is undefined for `x ≤ 0`, so the kernel raises
   `ErrFloatNan` for *every* negative base — but macOS libm `pow(-2, 3) = -8`,
   and the scalar man page documents only *fractional* negative-base exponents
   as having "no real result". Re-pointing scalar `pow` onto today's kernel
   would therefore **change** `math::pow(-2.0, 3.0)` from `-8` to an error.

The fix is to port fdlibm `__ieee754_pow`'s reconstruction (the proven strict +
fully-cased implementation):

- **Special cases first**, matching libm: `y == 0 → 1`; `x == 1 → 1`; `nan`
  propagation; `±0`, `±inf` bases/exponents per IEEE `pow`; `|x| == 1` with inf
  exponent, etc.
- **Sign / integer-exponent handling:** detect whether `y` is an integer and, if
  so, whether odd or even; compute `|x|^y` from the positive-base path and
  reapply the sign (`(-x)^y = ±|x|^y`). A non-integer `y` with `x < 0` keeps the
  current `ErrFloatNan` (matches both libm and the man page).
- **High-precision core:** `|x|^y = 2^(y·log2|x|)` with `log2|x|` and the
  `y·log2|x|` product carried in double-double (fdlibm's `t1/t2` split), then
  `2^(...)` via the `exp` kernel with the low correction (the existing
  `emit_exp_body_lo` low-tail seam already supports this).
- Overflow → `ErrFloatInf`, NaN result → `ErrFloatNan`, as today.

This affects **both** the array and scalar `pow` (they share the body), so the
array `pow` man-page line ("faithfully rounded … within 1 ULP for almost all
inputs") upgrades to the strict ≤1 ULP wording, and the array gains negative-base
+ integer-exponent support — an intentional, documented improvement that keeps
`f(x) == f([x])[0]`.

Update `gen_coeffs.py`'s `kpow` reconstruction to the double-double
`2^(y·log2 x)` form and (optionally) extend the capture set with negative-base
integer-exponent vectors so `pow.ref` exercises the new branch.

## Layout / ABI Impact

None to value layout, collection headers, copy/transfer, or golden output for
unrelated programs. The only ABI-adjacent change is the **import table**: after
Phase 5 the `native_call`/`link` math symbols (`pow`/`tan`/`atan2`/`fmod`) are
gone, so:

- Linux executables that previously listed `libm.so.6`/`libm.so.1` as needed and
  imported those four symbols no longer do (a program using only these may drop
  `libm` from its needed-library set entirely).
- macOS executables stop importing `_pow`/`_tan`/`_atan2`/`_fmod` from
  `libSystem` (libSystem stays for libc).
- `mfb spec linker import-selection` loses the `libm` row; `mfb spec linker
  linux-aarch64` loses the `libm.so` line; `mfb spec architecture native-ir`
  example loses the libm `link` entry.

These are documented import-surface reductions, not a binary-format change.

## Phases

1. **Scalar-binary seam + `atan2` re-point.** Add
   `lower_simd_float_binary_scalar` / `lower_math_scalar_binary`; route scalar
   Float `atan2` to the existing strict array kernel; drop its
   `lower_external_math` arm. *Acceptance:* `math::atan2(y,x)` emits no
   `branch_link` to `atan2`; `math::atan2(y,x) == math::atan2([y],[x])[0]`
   bit-for-bit; `atan2.ref` scores 100% ≤1 ULP via the kernel; existing
   `func_math_atan2_*` pass.
2. **`fmod` exact kernel + Float MOD re-point.** Implement `emit_float_fmod`;
   route `Float MOD Float` to it; remove `math.fmod` import collection.
   *Acceptance:* `Float MOD Float` emits no `branch_link` to `fmod`; result
   bit-identical to libm over a captured `fmod.ref` (incl. special cases);
   `arithmetic-float-mod-*` tests pass with unchanged observable behavior.
3. **`tan` strict ≤1 ULP + scalar re-point.** Generate `TAN_COEFFS` (if the
   `__kernel_tan` path is chosen), rewrite `emit_tan_body`, update the `ktan`
   reconstruction, re-point scalar `tan`. *Acceptance:* `tan.ref` scores 100%
   ≤1 ULP on the primary domain; scalar/array bit-identical; no `branch_link` to
   `tan`; `func_math_tan_*` pass.
4. **`pow` strict ≤1 ULP incl. negative-base-integer + scalar re-point.**
   Rewrite the `Pow` body (special cases, sign/integer handling, dd core), update
   `kpow`, extend the capture, re-point scalar `pow`. *Acceptance:* `pow.ref`
   (incl. new negative-base integer vectors) scores 100% ≤1 ULP; `math::pow(-2.0,
   3.0) == -8.0`; scalar/array bit-identical; no `branch_link` to `pow`;
   `func_math_pow_*` pass.
5. **Sever libm + doc/spec sync.** Delete the now-dead `pow`/`tan`/`atan2`/`fmod`
   rows from `linux_aarch64/plan.rs` and `macos_aarch64/plan.rs` (and the stale
   already-dead `sin`/`cos`/`exp`/… rows); add a codegen assertion / test that no
   `native_call` resolves to a platform math symbol. Update `mfb spec language
   builtin-functions` (pow/tan strict wording, "no external math library" now
   total), `mfb spec linker import-selection` + `linux-aarch64` (drop the `libm`
   row/line), `mfb spec architecture native-ir`, and the `math/{pow,tan,atan2}`
   man pages + the `MOD` operator docs. *Acceptance:* a program using all four
   surfaces imports zero libm/libSystem math symbols and no `libm.so` needed
   entry on Linux; `scripts/test-accept.sh` passes; `mfb spec`/`mfb man` build.

## Validation Plan

- **Function tests (full overload coverage):** extend/confirm
  `tests/func_math_pow_{valid,invalid}/**`, `func_math_atan2_{valid,invalid}/**`,
  `func_math_tan_{valid,invalid}/**` and their `_floatarray_*` siblings, plus
  `arithmetic-float-mod-{valid,invalid}/**`. Add cases that lock the new
  behavior: `pow(-2.0, 3.0) == -8.0`, `pow(-2.0, 0.5)` → `ErrFloatNan`,
  `tan` near `π/2`, `fmod` special cases (`MOD 0`, `|a|<|b|`, exact multiples).
- **Runtime accuracy proof (the real gate):** a runtime test that drives each
  kernel over its `tests/_data/math_kernel_ref/<fn>.ref` vectors and asserts
  every primary-domain lane is ≤1 ULP from the captured macOS value (`fmod`:
  0 ULP). This is the execution proof, not golden output — it proves the kernel,
  on hardware, matches macOS libm.
- **Pre-codegen check:** `python3 tools/math-kernels/gen_coeffs.py verify tan pow
  atan2` must show 100% / ≤1 ULP on the primary bucket once the reconstructions
  are upgraded (capture `fmod.ref` and, for `pow`, the negative-base vectors via
  `capture.sh`/`gen_inputs.py` first).
- **Cross-target identity:** run the same program on macOS, Linux-glibc, and
  Linux-musl (`.ai/remote_systems.md`); results bit-identical.
- **Import-surface proof:** inspect the emitted import table of a program using
  all four surfaces — zero libm/libSystem math symbols; no `libm.so` `DT_NEEDED`
  on Linux.
- **Doc sync:** `mfb spec language builtin-functions`, `mfb spec linker
  import-selection` + `linux-aarch64`, `mfb spec architecture native-ir`,
  `mfb spec diagnostics error-codes` (only if any error wording changes), and the
  affected man pages — kept current in the same commits.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Resolved Decisions

All four design forks are settled (confirmed 2026-06-27); the design above
reflects them.

- **pow negative-base + integer exponent** — **match libm fully.** `(-2)^3 = -8`;
  the kernel ports fdlibm's sign / integer-exponent logic (§4.4). Non-integer
  exponent with a negative base stays `ErrFloatNan` (matches libm and the man
  page). This both preserves scalar behavior on re-point and upgrades the array
  overload to the same semantics.
- **`tan` strict path** — **fdlibm `__kernel_tan` polynomial** (new `TAN_COEFFS`
  primitive), removing the near-pole divide for strict ≤1 ULP (§4.3).
- **`fmod` zero-divisor** — **keep the existing `ErrFloatDomain` pre-check** in
  `builder_numeric.rs`; the kernel is never entered with `b == 0` and needs no
  zero-divisor branch. MFBASIC has no NaN value, so libm's NaN path is
  unreachable by construction (§4.1).
- **Dead import rows** — **delete them.** Phase 5 removes the live
  `pow`/`tan`/`atan2`/`fmod` rows *and* the already-dead `sin`/`cos`/`exp`/…
  rows, so the goal — **no need to link libm at all** — is reached and the import
  tables stay honest (§Phase 5).

## Non-Goals

- Payne-Hanek large-argument trig reduction (same scope boundary as the shipped
  `sin`/`cos`; `tan` inherits the medium-range envelope).
- Any `Fixed` overload or `Fixed MOD` change.
- New `math::` overloads, names, or signatures.
- Touching libc symbols (`snprintf`, `getentropy`, `pthread_*`).

## Summary

The engineering risk is concentrated in two kernel bodies: **`tan`** (close a
near-pole 2-ULP residual to strict ≤1 ULP, recommended via fdlibm
`__kernel_tan`) and **`pow`** (reach strict ≤1 ULP *and* add fdlibm's sign /
integer-exponent / special-case logic so negative-base-integer matches libm —
a behavior change as much as an accuracy one). `atan2` is a pure re-point onto a
kernel that already validates strict, and `fmod` is a new but bit-exact integer
kernel. Everything reuses plan-01-simd's machinery — the array kernels, the
`broadcast → run body → extract lane 0` scalar seam, and the
`gen_coeffs.py verify` ↔ committed-reference-vector accuracy gate. Layout, ABI,
language surface, error codes, and `Fixed` paths are untouched; the visible
end-state is that **MFBASIC no longer imports any platform math symbol** —
Linux executables can drop `libm.so` entirely, and no `mfb` build links libm.

This makes the entire `math::` Float surface **internal, deterministic, and
bit-identical across every target** (macOS / Linux-glibc / Linux-musl): the
result of every transcendental, `pow`, `atan2`, `tan`, and `Float MOD` is now
produced by MFBASIC's own kernels rather than whichever platform libm the binary
happens to load — the same property `Fixed` already has, now extended to
`Float`.
