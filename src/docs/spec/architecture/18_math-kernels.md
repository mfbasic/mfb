# Math Kernels

Every `Float` and `Fixed` math result MFBASIC produces is computed by a
hand-written **in-tree kernel** — no platform math library is ever linked or
called. The `Float` transcendentals (`exp`, `log`, `log10`, `sin`, `cos`, `tan`,
`atan`, `asin`, `acos`, `atan2`, `pow`) are NEON `f64` kernels; the `Float MOD
Float` operator (`fmod`) is a GPR integer kernel; the `Fixed` transcendentals are
deterministic Q32.32 routines. This topic specifies the accuracy and determinism
contract those kernels meet, how they are validated, and the one place the
behavior deliberately diverges from a system C library.

The lowering dispatch is [[src/codegen/builtins/math/gen_math.rs:lower_math_call]];
the `Float` NEON kernels live in
[[src/codegen/builtins/vector/builder_simd_float_math.rs]] (with Remez coefficients in
[[src/codegen/builtins/vector/simd_kernel_coeffs.rs]]), `pow` in
[[src/codegen/builtins/math/gen_pow.rs:emit_pow_scalar]], and `fmod` in
[[src/codegen/builtins/math/gen_fmod.rs:emit_float_fmod]]. The scalar and array
overloads of each function **share one kernel**, so `math::f(x)` and
`math::f([x])[0]` are bit-identical (`./mfb spec language builtin-functions`).

## Determinism contract

The kernels contain no platform `branch_link` to a math symbol — verified by the
`no_libm_math_imports` regression test, which every shipping backend carries —
[[src/target/macos_aarch64/plan.rs]], [[src/target/linux_aarch64/plan.rs]],
[[src/target/linux_x86_64/plan.rs]], and [[src/target/linux_riscv64/plan.rs]] — and
observable as the absence of `_pow`/`_sin`/`_fmod`/… in a built binary's import
table. Because the same hand-written code runs on every target and uses only IEEE
`f64` operations (with FMA where a kernel's reduction calls for it), **a math
result is bit-identical on macOS, Linux-glibc, and Linux-musl** — the same
property `Fixed` already had, now extended to `Float`. There is no
reduction-order ambiguity, no libm version skew, and no last-ULP platform drift.

This extends to the x86_64 and riscv64 backends, and to ordinary user-level
`Float` `a*b±c` expressions, because scalar fused multiply-add is decided in
target-neutral MIR lowering and IEEE-754 FMA is a correctly-rounded,
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
vectors in `tools/math-kernels/ref/<fn>.ref`. The kernels meet it as follows:

| Function(s) | Bound | Notes |
|---|---|---|
| `exp`, `log`, `log10`, `sin`, `cos`, `atan`, `asin`, `acos`, `atan2` | **≤1 ULP** of macOS libm | double-double-compensated Remez polynomials; fdlibm 4-segment `atan`; `acos` via the half-angle identity `2·atan(√((1−x)/(1+x)))`; `sin`/`cos` at any finite angle, see *Trig range reduction* |
| `pow` | **≤1 ULP** of macOS libm | fdlibm `__ieee754_pow` in log2 space; negative base with an integer exponent matches libm (`(-2)^3 = -8`) |
| `tan` | **faithfully rounded — ≤1 ULP of the TRUE value**, at any finite angle | more accurate than macOS libm; see below |
| `fmod` | **0 ULP — bit-identical** to libm | the IEEE remainder is exactly representable |
| `Fixed` `sin`/`cos`/`tan` | **≤1 Q32.32 unit (2^-32) of the TRUE value**, at every representable argument | exact 160-bit `pi/2` reduction + Q1.63 Taylor series; `tan` raises `ErrOverflow` when the true tangent leaves the `Fixed` range (bug-615) |
| `Fixed` `asin`/`acos`/`atan`/`atan2` | **≤1 Q32.32 unit (2^-32) of the TRUE value**, over the whole domain | one `atan(num/den)` kernel on an exact integer ratio + Q1.63 Taylor series, rounded once (bug-615); measured maximum 0.4990 units over an 855-argument corpus |
| other `Fixed` transcendentals, `Fixed MOD` | deterministic Q32.32 | platform-independent by construction; not an `f64` bound |

Both trig bounds hold at **every finite angle**. The regression test
`tests/runtime/rt_math_float_trig_large_angles.rs` scores scalar and `List OF
Float` `sin`/`cos`/`tan` against an exact 1400-bit big-integer oracle over 410
angles — the ordinary range, the `2^20*pi/2` edge, `2^52`, `2^53`, `1e20`,
`1e22`, `1e100`, `1e300`, `f64::MAX`, the double closest to a multiple of pi/2
(`6381956970095103*2^797`, whose cosine is about `4.7e-19`), and 360 random
doubles with exponents in `[20, 1023]` — and requires one ULP of the true value
on every one, with scalar and `List` bit-identical.

### `Fixed` trigonometry

`Fixed` is Q32.32, so its unit is `2^-32` and its range ends near ±2.1e9. The
`sin`/`cos`/`tan` kernels are integer-only (no host `f64` anywhere, including the
baked constants, so every target agrees bit for bit) and reduce the angle against a
**160-bit `pi/2`**, which keeps the reduction error below `2^-127` for every `k`
below `2^31`. Before bug-615 the reduction used a `pi/2` rounded to Q32.32 and lost
`k * 0.26` units — `sin(toFixed(2000000000.0))` was 0.9432 against a true 0.9147 —
and a 31-step Q32.32 CORDIC left several units of residue even at tiny angles
(`cos(0)` read 1.00000000163). The reduced angle now feeds a Q1.63 Taylor series,
rounded once to Q32.32.
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_trig_reduce]]
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_sin_cos_magnitudes]]

`tan` is the quotient of those Q1.63 magnitudes (`cot` in an odd quadrant), and
close to a pole — an odd quadrant with `|r| < 2^-8` — it is evaluated as
`cot r = 1/r − r/3 − r^3/45` from a 128-bit long division, because one-unit accuracy
at a tangent near `2^31` needs about 95 bits of the reduced angle. A true tangent
outside the `Fixed` range raises `ErrOverflow` (`77050010`), declared on the `Fixed`
overload only — the `Float` overloads return a large finite value there and cannot
overflow. [[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_tan]]
[[src/codegen/builtins/math/func_tan.rs:register]]

### `Fixed` inverse trigonometry

`asin`, `acos`, `atan` and `atan2` on a `Fixed` share one kernel,
`atan(num/den)` for two unsigned 64-bit words, returning an unsigned Q3.61 angle
in `[0, pi/2]`. Taking a **ratio** rather than a `Fixed` is what buys the
accuracy: the quotient is never rounded to Q32.32 first, so a huge or a tiny
argument keeps its full relative precision.
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_atan_ratio]]

Two range folds are exact integer rewrites of the ratio — `num > den` swaps the
words for `atan t = pi/2 − atan(1/t)`, and `2·num > den` replaces `(num, den)` with
`(den − num, den + num)` for `atan t = pi/4 − atan((1−t)/(1+t))`. Both leave
`u = num/den ≤ 1/2`, an exact unsigned Q0.63 quotient from a 63-step long
division, which a 28-level Horner Taylor series `atan u = u·Σ(−u²)ⁿ/(2n+1)`
evaluates in Q1.63; the first omitted term is `u^55/55 < 2^-60`. The angle
therefore carries about 60 bits and the whole family rounds **once**, at the
Q3.61 → Q32.32 step, so the measured error is rounding-limited.
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_q61_to_signed_fixed]]

`atan2`'s axes are exact baked constants (`atan2(0, x<0)` is `+pi`, not `-pi`);
elsewhere the quadrant is added in Q3.61 before that rounding step, `x < 0`
taking `pi` minus the first-quadrant angle and the sign of `y` applied last.
`atan(x)` is `atan2(x, 1.0)`.
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_atan2]]

`asin x = atan(x / sqrt(1 − x²))`, again as an integer ratio. With `x = r/2^32`,
`1 − x² = d/2^64` for `d = 2^64 − r²`, which is exactly the low word of `−r·r`
because `|r| ≤ 2^32`; the ratio is `|r|·2^16 / round(sqrt(d)·2^16)`, the root
being the Q32.32 square-root kernel applied to `d` read as an unsigned word. That
is what survives `|x| → 1`: a Q32.32 `sqrt(1 − x²)` keeps one or two bits there,
while this ratio stays exact in the numerator and a half-unit root error moves the
angle by at most `2^-48` radians. `|x| = 1` gives `d = 0`, and a zero denominator
answers `pi/2` with no divide. Unlike the `Float` kernel, the `Fixed`
`acos x = pi/2 − asin x` is formed in Q3.61 **before** the single rounding step,
so the cancellation that forces the `Float` half-angle identity never reaches a
rounded operand. [[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_asin]]

Before bug-615-C all four ran a 31-step Q32.32 CORDIC **vectoring** loop and kept
its residue: `atan(2.0F)` was 4.27 units out, `atan2(3.0F, -4.0F)` 2.69,
`asin(0.8F)` 1.61. The loop has no callers left and is deleted.

### `Fixed` `exp` and fractional `pow`

`exp` on a `Fixed` is `2^n · exp(r)` with `n = round(x / ln2)` and `r = x − n·ln2`,
the reduced `exp(r)` coming from an 18-term Taylor series and the `2^n` recombination
from a doubling/halving loop that raises `ErrOverflow` on the way up.
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_exp]]
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_scale_by_power_of_two]]

That recombination **dispatches on the sign of `n`** — non-negative doubles with the
overflow check, negative halves without one — so the reduction's sign is load-bearing
in a way its magnitude is not. `x / ln2` is an unchecked Q32.32 multiply, and past
`|x| = 2^31·ln2 ≈ 1.4885e9` it left `Fixed` range and wrapped, handing the loop a
sign-flipped `n`: that selected the opposite arm and skipped the overflow check
entirely, so overflow and underflow silently swapped (bug-659). The kernel therefore
gates the argument **before** reducing it. `Fixed` spans just under `[−2^31, 2^31)`,
so the result overflows above `ln(2^31) = 21.4876` and rounds to zero below
`−33·ln2 = −22.8742`; the gate sits at `|x| = 64`, outside both thresholds (no
in-range result moves) and far inside the wrap point (the reduction can no longer
leave range). Above `+64` the kernel raises `ErrOverflow` (`77050010`) and below
`−64` it returns `0.00`, decided from the sign of `x` itself.

`pow` on a `Fixed` with a **fractional** exponent is `exp(exponent · ln(base))`, which
inherits that sensitivity one level up: `|ln(base)|` reaches about 22 across the
`Fixed` domain, so an exponent past roughly `1e8` drives the product itself out of
Q32.32 range. A wrapped product flips the sign `exp`'s gate then reads its answer off,
so that multiply **saturates** to `i64::MAX`/`i64::MIN` instead of wrapping — bit-identical
to the plain multiply in range, and past the gate in the correct direction outside it,
which is the true result since `|exponent · ln(base)| > 2^31` cannot land inside `Fixed`
range. The whole-number-exponent path is a separate exact multiply loop and never
reaches either seam.
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_mul_saturating]]
[[src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_pow_general]]

### The `Float` `acos` half-angle identity

The **`Float`** `acos` kernel deliberately uses the half-angle identity rather than `π/2 − asin(x)`: the
latter cancels catastrophically as `x → +1` (where `acos → 0`), while `1±x` is
exact for `|x| ≤ 1` (Sterbenz), so `2·atan(√((1−x)/(1+x)))` stays ≤1 ULP across the
whole domain. The endpoints fall out of IEEE arithmetic — at `x = −1` the divide
yields `+inf`, `atan(+inf) = π/2`, and `2·(π/2) = π` exactly.

## Trig range reduction

`sin`, `cos` and `tan` share one reduction of the angle to `r` in `[-pi/4, pi/4]`
plus a quadrant [[src/codegen/builtins/vector/builder_simd_float_math.rs:emit_sincos_reduce]].
It has two paths, chosen **per lane**, and it yields `r` as a double-double: the
high half drives the compensated Horner polynomials, and the low half enters them
as the first-order corrections `r_lo*(1 - r^2/2)` on `sin(r_hi)` and `-r_hi*r_lo`
on `cos(r_hi)` — without which the last ULP is lost even in the ordinary range
(bug-618 measured 1.29 ULP at `sin(16.223429914714487)` and 1.52 at
`tan(417085.1813766881)` before the low half was carried).

* **Medium** — fdlibm's three-part Cody-Waite step (`PIO2_1`/`PIO2_2`/`PIO2_2T`),
  one multiply-subtract per part. `q*PIO2_1` and `q*PIO2_2` are exact only while
  `|x| < 2^20*pi/2`, and the three constants truncate `pi/2` at about 120 bits,
  leaving an absolute error near `3e-37*|x|`.
* **Exact (Payne-Hanek)** — taken when `|x| >= 2^20*pi/2`, or when the medium
  step's angle cancelled below `2^-50*|x|` (where that `3e-37*|x|` could reach the
  result's last ULP)
  [[src/codegen/builtins/vector/builder_simd_float_math.rs:emit_rem_pio2_large_lane]].
  Write `|x| = m*2^e`: the bits of `2/pi` worth `2^-i` with `i <= e-2` only add
  multiples of 4, so a 256-bit window of `2/pi` starting at bit `e-1` decides both
  the quadrant and the fraction. The window is four words of a baked 1225-bit
  `2/pi` table indexed by the exponent
  [[src/codegen/builtins/vector/builder_simd_float_math.rs:TWO_OVER_PI_WORDS]];
  `m` times the window is three 64x64→128 integer products (`mul`/`umulh`) and a
  carry chain; the product shifted by the window's sub-word offset puts the
  integer part mod 4 in its top two bits and at least 190 fraction bits below
  them, with the untabulated tail of `2/pi` contributing under `2^-138`. Rounding
  that fraction to the nearest integer gives the quadrant and a signed fraction
  whose magnitude, split into three exactly-convertible 53-bit pieces and scaled
  by the double-double `pi/2`, is the reduced angle to about `2^-100` relative —
  enough for the closest double to a multiple of pi/2 (`6381956970095103*2^797`,
  reduced angle `4.7e-19`), since no double comes within `2^-61` of one.

The table is committed as integer constants generated offline in exact integer
arithmetic (Machin's formula, no host floating point); the generator is recorded
in the constant's own doc comment, and its leading words match fdlibm's
`two_over_pi`. It lives at the end of the math constant pool and is addressed by
index (`pool + table + 16*j`), which is why it is the one part of the pool that is
neither deduplicated nor reachable by value
[[src/codegen/builtins/vector/builder_simd_float_math.rs:math_const_pool_words]].

The exact path is branched around unless a lane needs it, so an ordinary call pays
only the routing compare. A two-lane array call whose lanes disagree runs the
exact reduction for the routed lane(s) and selects per lane, so the scalar and
`List OF Float` overloads stay bit-identical. NaN and infinite lanes are never
re-reduced: their medium result is NaN and raises `ErrFloatNan` exactly as before.

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
   `tools/math-kernels/ref/` *are* macOS libm.
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

Both tools predate the exact large-argument reduction and still bucket trig
vectors above `2^20*pi/2` as "extended … out of scope"
(`tools/math-kernels/README.md` §verify scope, `runtime_ulp.py:_is_primary`); the
codegen no longer is, and the standing gate for that range is the runtime test
named under *Accuracy* above.

See `tools/math-kernels/README.md` for the full tooling, and
`./mfb man math` for the per-function user documentation.

## See Also

* ./mfb spec language builtin-functions — the user-facing `math::` member list and accuracy prose
* ./mfb spec linker import-selection — why no build links `libm`
* ./mfb spec architecture aarch64-instruction-set — the NEON `CodeOp`s the kernels emit
* ./mfb spec diagnostics error-codes — `ErrFloatDomain`/`ErrFloatNan`/`ErrFloatInf`/`ErrInvalidArgument`
