# bug-618: `math::sin` / `cos` / `tan` return impossible values for very large `Float` angles

Last updated: 2026-09-13
Effort: small–medium
Severity: MED
Class: Correctness (silent wrong result)

Status: Open
Regression Test: none yet — see Phase 1

A sine or cosine is always within `[-1, 1]`. For a very large finite `Float`
angle, `math::sin` and `math::cos` return numbers many orders of magnitude outside
that range, and raise nothing:

| `x` | `math::sin(x)` | `math::cos(x)` | `math::tan(x)` |
|---|---|---|---|
| `1e6` | -0.349994 | 0.936752 | -0.373624 |
| `1e7` | 0.420548 | -0.907270 | -0.463531 |
| `1e9` | 0.545843 | 0.837887 | 0.651452 |
| `1e20` | **196250469054002088973237747712.000000** | **-3671613044105074559982501888.000000** | -53.450749 |

Every row up to `1e9` is plausible. At `1e20` the `sin` and `cos` results are
impossible, and `tan` is probably wrong too (its value is not checked against a
reference here).

**The single correct behavior a fix produces:** for every finite `Float` `x`,
`math::sin(x)` and `math::cos(x)` lie in `[-1, 1]`, and all three agree with a
correctly-rounded reference (e.g. MPFR or a libm oracle) to the package's stated
accuracy. That includes angles so large that adjacent `Float` values are more than
2·pi apart.

Found by plan-125-C Phase 1's Codex page review of `math::sin`
(`planning/plan-125-findings/C-phase1/man-page-math-sin.md`, finding 1). The
reviewer cited the Cody–Waite reduction in
`src/codegen/builtins/vector/builder_simd_float_math.rs` as accurate only below
`2^20 * pi/2`. Confirmed with the release binary (`/tmp/p125-ex/mathc1d`).
**Filed, not fixed**, by user instruction during a documentation-only plan
("file all bugs, make no fixes").

## Reproduction

```basic
IMPORT io
IMPORT math

SUB main()
  io::print(toString(math::sin(100000000000000000000.0), toByte(6)))
  io::print(toString(math::cos(100000000000000000000.0), toByte(6)))
END SUB
```

Observed (macos-aarch64, `worktree-P-125`): `196250469054002088973237747712.000000`,
`-3671613044105074559982501888.000000`. Expected: two values in `[-1, 1]`.

## Root cause

Hypothesis, from the reviewer's citation, to confirm in Phase 1: the Float kernel
reduces the angle modulo pi/2 with a fixed-precision Cody–Waite step
(`builder_simd_float_math.rs`), which is only accurate while the quotient `n`
fits the precision the step assumes (below `2^20 * pi/2`, about 1.6e6). Past that,
the reduced remainder is not within `[-pi/4, pi/4]`, and the polynomial
evaluation is used far outside its interval, so it diverges. The `1e6`–`1e9` rows
still look plausible, so the practical break-point is somewhere above `1e9`. Phase
1 bisects it, and checks the middle rows against a reference, since "plausible"
is not "correct".

### Confirmed (Phase 1, 2026-09-17)

Reproduced on macos-aarch64 at `798870ec2`. RED test
`tests/runtime/rt_math_float_trig_large_angles.rs` checks scalar and two-lane
`List OF Float` sin/cos/tan against an exact 1400-bit big-integer oracle over 410
angles in `[2^20, f64::MAX]` (plus ordinary angles): 2232 results are wrong.

- Every ordinary angle, including the fdlibm medium-range edge `2^20 * pi/2` and
  1e9, is within one ULP of the truth — the in-range kernel is fine.
- The first failures appear near 2e11 (just past one ULP); `sin(2^52)`/`sin(2^53)`
  are wrong in the eighth digit; 1e20 and 1e22 return impossible magnitudes
  (`sin(1e22)` = 4.98e74).
- **From about 1e100 upward every call raises `ErrFloatNaN` (77050013)** instead of
  returning a value — `q = x * 2/pi` and `q * PIO2_1` stop cancelling. A whole
  `List OF Float` call fails if any lane holds such an angle.

Mechanism: `builder_simd_float_math.rs:emit_sincos_reduce` is the fdlibm three-part
Cody-Waite step (`PIO2_1`/`PIO2_2`/`PIO2_2T`), exact only while `q` fits `PIO2_1`'s
33 bits, followed by `fcvtzs(q) & 3` for the quadrant, which saturates for huge `q`.
`tan` shares the reduction (`emit_tan_sincos_dd`).

Fixed audit: the Fixed overloads use a different reduction and are broken in their
own right — see bug-615 (sub-issue 615-B).

## Non-goals

- Changing results for ordinary angles.
- Documenting the garbage as behavior. Until the fix, `mfb man math sin`/`cos`
  says nothing about huge angles beyond what is verified.

## Blast-radius audit

- `sin`, `cos`, `tan`, both scalar `Float` and `List OF Float` (shared kernel).
- The `Fixed` overloads use a separate lowering (`gen_fixed_math.rs`), and
  `Fixed`'s range (±2.1e9) never reaches the broken region; audit them anyway.
- `vector::` or any other member built on the same trig kernel.

## Fix

Phase 1 — RED tests against a reference at `1e20`, `2^53`, and a bisected
threshold; locate the reduction step. Commit:

Phase 2 — exact reduction for large arguments (e.g. the fdlibm multi-word
`__ieee754_rem_pio2` approach) (GREEN); goldens for the trig kernel; full suite.
Commit:
