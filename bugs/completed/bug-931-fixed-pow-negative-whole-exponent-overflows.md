# bug-931: `math::pow(Fixed, Fixed)` raises ErrOverflow for a negative whole exponent whose result is tiny (`pow(2.0F, -31.0F)`, `pow(10.0F, -10.0F)`)

Last updated: 2026-10-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/math/fixed-pow-negative-whole-exponent-small-result-rt

## Cause (coordinator, bug-staging-24)

- Rule: `math::pow(Fixed, Fixed)` with a negative whole exponent returns the Fixed nearest `base^n` (0.00 once it underflows), never `ErrOverflow`; `ErrOverflow` only when the true result is out of range (the doc's Correct behavior).
- Where: `src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_pow_general`, integer-exponent branch: `|n|` checked multiplies, then the reciprocal, so an intermediate `base^k >= 2^31` traps before the reciprocal brings it back.
- Fix: for `n < 0` the magnitude `base^|n|` is not range-checked as a Fixed: it is computed in a wider intermediate (or saturated: past range its reciprocal rounds to 0) and the reciprocal is rounded once to the nearest Q32.32; the overflow trap stays only on the result.

`math::pow` on `Fixed` with a **whole-number negative** exponent computes the forward power
`base^|n|` with the overflow-trapping Q32.32 multiply and only then takes the reciprocal. When
`|base| > 1` and `base^|n|` leaves the Fixed range (>= 2^31), the forward multiply raises
`ErrOverflow` (77050010) even though the true result `base^-|n|` is a small number that is
in range — sometimes exactly representable. `pow(2.0F, -31.0F)` is exactly 2^-31 = two Q32.32
units (`0.000000000466`), yet it raises. `pow(2.0F, -30.0F)` works (`0.000000000931`), because
2^30 still fits.

The fractional path gets the same region right: `pow(2.0F, -40.5F)` returns `0.000000000000`
(spec: below the representable range the result rounds to zero, as `exp` does below -64), while the
neighbouring whole exponent `pow(2.0F, -40.0F)` raises. So the result depends on whether the
exponent has fractional bits, not on the value.

**Correct behavior:** a negative whole exponent with `|base| > 1` returns the Fixed value nearest
`base^n` (which underflows to `0.00` once `|base^n| < 2^-33`), never `ErrOverflow`. `ErrOverflow`
stays correct only where the true result is out of range: `|base| < 1` with a large negative
exponent (`pow(0.5F, -40.0F)` = 2^40, the bug-137 case), and zero to a negative power.

References:

- `src/docs/spec/architecture/18_math-kernels.md` "`Fixed` `exp` and fractional `pow`" (below
  -64 `exp` "returns `0.00`"; "The whole-number-exponent path is a separate exact multiply loop").
- `mfb man math pow`: "a negative exponent gives a reciprocal (pow(2.0, -2.0) is 0.25) ... A result
  too large to hold — including zero to a negative power — raises ... ErrOverflow for Fixed." A
  result of 2^-31 is not "too large to hold".
- Related, distinct: bugs/completed/bug-137-math-codegen-low-cluster.md §3 (the `|base| < 1`
  underflow-then-reciprocal case, which correctly raises ErrOverflow).
- Found in hardening batch B12 (architecture/18_math-kernels), Fixed kernel sweep.

## Failing Reproduction

```
IMPORT io
IMPORT math
FUNC main() AS Nothing
  LET b AS Fixed = 2
  LET e AS Fixed = -31
  io::print(toString(math::pow(b, -30.0F), toByte(12)))
  io::print(toString(math::pow(b, e), toByte(12)))
END FUNC
```

`mfb build` then run (macOS aarch64, /tmp/hardening/mfb at HEAD):

- Observed:
  ```
  0.000000000931
  Error: 7-705-0010
  Arithmetic overflow or numeric conversion outside the destination range.
  ```
  exit 255.
- Expected: `0.000000000931` then `0.000000000466` (2^-31 exactly), exit 0.

Measured matrix (`show(b, e)` wrapper printing `pow` or the trapped code):

| call | observed | true value |
| --- | --- | --- |
| `pow(2, -30)` | 0.000000000931 | 2^-30 |
| `pow(2, -31)` | raised 77050010 | 2^-31 (2 units, exact) |
| `pow(2, -32)` / `-33` / `-40` / `-64` | raised 77050010 | 2^-32 (1 unit) / underflow -> 0 |
| `pow(10, -10)` / `pow(10, -11)` | raised 77050010 | 1e-10 (0.43 unit) -> 0 |
| `pow(2, -40.5)` (fractional) | 0.000000000000 | underflow -> 0 (correct) |
| `pow(0.5, 33)` / `pow(0.5, 64)` | 0.000000000000 | underflow -> 0 (correct) |
| `pow(0.5, -40)` | (bug-137) raises 77050010 | 2^40 out of range (correct) |

## Root Cause

`src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_pow_general`, integer-exponent branch:
the loop `result = emit_fixed_multiply(result, base)` runs `|n|` times regardless of the sign of
`n`, and `emit_fixed_multiply` traps `ErrOverflow` as soon as the running product leaves Q32.32
range. The reciprocal (`emit_fixed_divide(1.0, result)`) is applied only after the loop
(`fixed_pow_int_nonneg` label), so for `n < 0` and `|base| > 1` an intermediate `base^k >= 2^31`
traps before the reciprocal would have brought the value back into range.

## Goal

- For `n < 0`, `|base| > 1`: `pow` returns the nearest Fixed (or 0.00 once below half a unit) and
  never traps. E.g. iterate on the reciprocal (`r = 1/base`, then multiply `r` `|n|` times with
  the existing truncate-to-zero early exit), or detect forward overflow and return 0 when the
  exponent is negative. Accuracy for in-range results must not regress (`pow(3.0F, -5.0F)`).

### Non-goals (must NOT change)

- `pow(0.5F, -40.0F)` and `pow(0.0F, -1.0F)` keep raising `ErrOverflow` (true result out of range).
- Positive-exponent overflow (`pow(2.0F, 31.0F)`) keeps raising `ErrOverflow`.
- The `^` operator path (`builder_numeric::emit_fixed_pow`) rejects negative exponents by design;
  untouched.

## Blast Radius

- `gen_fixed_math.rs:emit_fixed_pow_general` integer branch — fixed by this bug.
- `builder_numeric::emit_fixed_pow` (`^` operator) — unaffected: rejects negative exponents.
- Float `pow` (`gen_pow.rs:emit_pow_scalar`) — unaffected: `pow(2.0, -1074.0)` returns the subnormal
  correctly (measured in the B12 Float sweep, 0 ULP).

## Fix (bs24-agent-f, 2026-10-18)

**Correction to the reproduction's expected text.** `toString(x, toByte(12))` on a
`Fixed` truncates, so 2^-31 (exactly two Q32.32 units, 4.6566e-10) prints
`0.000000000465`, not `0.000000000466`; the in-range `pow(-2.0F, -31.0F)` already
printed `-0.000000000465` before this fix. The value is right; the doc's text was not.

**Fix.** In `emit_fixed_pow_general`'s integer-exponent loop the multiply is
`emit_fixed_multiply_or_branch` (the non-raising core `emit_fixed_multiply` now wraps,
emitting the same code). A forward power that leaves the `Fixed` range raises
`ErrOverflow` only for `n ≥ 0`; for `n < 0` it goes to `emit_fixed_pow_tiny_reciprocal`,
which carries the rest of the power as an unsigned Q36.28 magnitude (no range check
as a `Fixed`), returns 0 once the power passes 2^34, and rounds `2^60 / power` once
to the nearest unit (ties away), with the sign of `base` for an odd `|n|`. The
in-range path (truncating reciprocal) is unchanged; `pow(0.5F, -40.0F)`,
`pow(0.0F, -1.0F)` and `pow(2.0F, 31.0F)` still raise `ErrOverflow`.

**RED → GREEN.** `bash scripts/test-accept.sh <mfb> <out>
'rt-behavior/math/fixed-pow-negative-whole-exponent-small-result-rt'`: main's compiler
mismatches (every tail case `RAISED 77050010`); this branch passes: `pow(2,-31)` =
`0.000000000465` (2 units), `pow(2,-32)` / `pow(2,-33)` / `pow(3,-20)` /
`pow(1.5,-55)` = 1 unit, `pow(2,-34)` / `-40` / `-64` / `pow(10,-10)` / `-11` /
`pow(2,-2000000000)` = `0.000000000000`, `pow(-2,-32)` = 1 unit, in-range lines
unchanged. Same output at `-O0` and `-O3`.

**Goldens expected to move.** Every `.ncode`/`.ncodesum` of a program that calls
`math::pow` on `Fixed` (the loop's emitted code and label numbering change) — at least
`tests/byte-identity/math` (artifact kind `.ncodesum`, all five targets), reason: the
new tail and branch. Measured: `bash scripts/test-fast.sh --artifact math --accept
'<this fixture>'` — `artifact-math` reports exactly the five
`byte-identity/math/math_codegen_cover_rt.<target>.ncode` sha256 diffs (the fixture
calls `math::pow(Fixed, Fixed)`); every other step ok. `emit_fixed_multiply`'s own
code is byte-identical: `scripts/artifact-gate.sh <mfb> money` (Fixed multiplies, no
`pow`) reports 0 diffs.
