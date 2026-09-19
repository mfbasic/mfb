# bug-618: `math::sin` / `cos` / `tan` return impossible values for very large `Float` angles

Last updated: 2026-09-19
Effort: small–medium
Severity: MED
Class: Correctness (silent wrong result)

Status: Fixed (Phase 2) — pending the integration full suite / golden refresh
Regression Test: `tests/runtime/rt_math_float_trig_large_angles.rs`

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

Phase 2 — exact reduction for large arguments (GREEN); goldens for the trig
kernel; full suite. Commits: `94be9af2c` (kernel), `314e033ff` (spec),
`62a459532` (man pages).

`emit_sincos_reduce` now runs the fdlibm medium Cody-Waite step and then, **per
lane**, replaces its result with an exact reduction when the medium step cannot
be trusted to the last ULP:

- `|x| >= 2^20 * pi/2` (the step's products stop being exact), or
- the medium reduced angle cancelled below `2^-50 * |x|` (the three truncated
  `pi/2` constants leave an absolute error near `3e-37 * |x|`, which at that much
  cancellation reaches the result's last ULP).

The exact path (`emit_rem_pio2_large_lane`) is Payne-Hanek over a fixed-width
window. `|x| = m * 2^e`: bits of `2/pi` worth `2^-i` with `i <= e-2` only add
multiples of 4, so the window starts at bit `e-1`, which after the table's 55-bit
pad is table bit `s = biased_exponent - 1022` — word `s >> 6`, offset `s & 63`.
`m` times the four-word (256-bit) window is three `mul`/`umulh` pairs plus a
carry chain; shifted left by `s & 63`, the top two bits are the quadrant and at
least 190 bits below them are the fraction (untabulated tail < `2^-138`).
Rounding the fraction picks the quadrant and the sign, one's complement gives the
magnitude, three exactly-convertible 53-bit pieces of it sum to a double-double,
and the double-double `pi/2` scales it to radians — about `2^-100` relative,
against a worst case (`6381956970095103 * 2^797`) of `2^-61.5`. The `2/pi` table
is 20 committed 64-bit words (`floor(2/pi * 2^1225)`), generated offline in exact
integer arithmetic by the recipe in its doc comment, appended to the math
constant pool and addressed by index.

Both paths hand the evaluators a double-double angle: the low half enters the
compensated Horner as `r_lo*(1 - r^2/2)` on `sin(r_hi)` and `-r_hi*r_lo` on
`cos(r_hi)`.

**A second defect found while fixing this one, and fixed with it.** The medium
path discarded the low half of its own reduction, which costs the last ULP well
inside the "medium" range the spec claims at <=1 ULP. Measured against the same
exact oracle, at `798870ec2`: `sin(16.223429914714487)` 1.29 ULP,
`cos(842522.8010648803)` 1.39, `tan(417085.1813766881)` 1.52 (and
`tan(413441.44719405076)` 2.17, a near-multiple of pi/2 that is now routed to the
exact path). The gate test's corpus does not contain those angles — it was
written for the large-angle failure — so they are worth adding to it or to a
sibling test. Consequence: results for `pi/4 <= |x| < 2^20*pi/2` move in the last
bit where the low half now counts (about 20% of sampled angles); results for
`|x| < pi/4` are unchanged (the reduction is a no-op there). math/vector
`.ncodesum` goldens and any `.run` golden printing a trig value at full precision
move with it.

Verification (macos-aarch64, `target/debug/mfb`):

- `cargo test --test rt_math_float_trig_large_angles` — `2 passed`, was 2232
  wrong results.
- `tools/math-kernels/runtime_ulp.py tan` — primary 907 vectors 100.00% <=1 ULP
  vs TRUTH (maxULP 1; macOS libm itself >1 ULP on 19 of them), and the 20
  "extended" large-argument vectors the tool still calls out of scope are now
  100.00% <=1 ULP vs macOS.
- `runtime_ulp.py exp|log|atan2 --limit 300` — 100.00% <=1 ULP vs truth, i.e.
  appending to the math constant pool moved no other kernel.
- `cargo test --bin mfb math` / `vector` / `spec` / `no_libm` green;
  `scripts/spec-census.sh --citations` 0 misses; `scripts/man-census.sh
  --memory-scope` 0; `scripts/man-run-examples.sh math --run` 21/21.
- All five backends build a trig program (`-target macos-aarch64`,
  `linux-aarch64`, `linux-x86_64`, `linux-riscv64`, `windows-x86_64`). Only the
  macOS host was executed here; the Linux/Windows/riscv64 runtime proof is the
  integration run.

Consumer audit (everything that reaches this reduction):

- `math::sin`/`cos`/`tan`, `Float` scalar and `List OF Float` — both overloads
  share the kernel, and the fix keeps them bit-identical (the gate test checks
  every result both ways).
- `vector::rotate_2d` (`Float2`) and `vector::slerp` (`Float2/3/4`) call
  `math::sin`/`math::cos` on `Float` in their generated bodies, so they inherit
  the fix; their `Fixed`/`Integer` forms use the `Fixed` kernel instead.
- `atan`/`asin`/`acos`/`atan2` reduce by segment, not by pi/2, and are untouched;
  `exp`/`log`/`log10`/`pow`/`fmod` have their own reductions.
- Nothing folds `math::sin`/`cos`/`tan` at compile time (no `f64::sin` anywhere in
  `src/`), so there is no second implementation to keep in step.
- The riscv64 RVV dual path is a lowering of the same target-neutral ops, not a
  second kernel; `tools/math-kernels/rvv-*` only score it. Those two tools, and
  `tools/math-kernels/README.md`, still describe large-argument trig as
  out-of-scope Payne-Hanek (`runtime_ulp.py:_is_primary`, `gen_coeffs.py`'s
  `extended` bucket): stale now, and worth a follow-up so the large-angle vectors
  count toward the gate rather than an excluded bucket.

Also noticed, not fixed: `math::sin(-0.0)` returns `+0.0` (IEEE sine preserves the
zero's sign). Pre-existing, unchanged by this fix, and separate from the
reduction.
