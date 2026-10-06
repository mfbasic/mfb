# bug-878: `toString(Fixed, n)` does not round the exact Q32.32 value: it truncates at n >= 10 (not "trailing zeros") and over-rounds just below a midpoint at n < 10

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness (owner-confirmed: exact rounding + inexact-literal warning)

Status: Open
Regression Test: tests/rt-behavior/conversions/fixed-tostring-exact-rounding-rt (new) — name suggested

**Owner decision (2026-10-05):** round the exact stored value, and warn on inexact Fixed literals.
1. `toString` of a `Fixed` rounds the **exact Q32.32 value** half-away-from-zero at every
   precision. The `ceil(2^31 / 10^n)` pre-round bias is removed. So `toString(1.005F)` is
   `1.00`, `2.675F` → `2.67`, and at ≥10 places the true digits are rounded, not truncated
   (`toString(toFixed(1) / toFixed(3), toByte(10))` → `0.3333333333`). The spec sentence about
   "trailing zeros" is corrected.
2. **New compile-time warning, Fixed only.** A `Fixed` literal whose decimal spelling is not
   exactly representable in Q32.32 (e.g. `1.005F`, `0.1F`) gets a located warning stating
   the value actually stored (`1.005F` is stored as `1.00499999988824…`), so the developer
   knows what they wrote is not what they get. Model it on `TYPE_MONEY_LITERAL_PRECISION`
   (2-203-0108, warn). It is a new rule, e.g. `TYPE_FIXED_LITERAL_INEXACT`, added to the
   diagnostics registry and the build-input error-code table as needed. It applies to
   `F`-suffixed literals and decimal literals that acquire `Fixed` from an expected type. It
   **never** fires for `Float`: Float is not expected to be exact.

The spec says `toString` of a `Fixed` renders `decimals` places "of the exact Q32.32 value rounded
half away from zero; at 10 or more places the extra digits are trailing zeros". Neither half holds:

1. **n >= 10:** the extra digits are not zeros — a Q32.32 fraction has up to 32 exact decimal
   digits — and they are **truncated**, not rounded: `toString(toFixed(1) / toFixed(3), toByte(10))`
   is `0.3333333332` (exact value 0.33333333325572…, which rounds to `0.3333333333`);
   `toString(1.005F, toByte(20))` is `1.00499999988824129104`.
2. **n < 10:** the bias is `ceil(2^31 / 10^n)` raw units, up to ~0.52 ULP more than half, so a value
   that lies strictly *below* the decimal midpoint rounds up: `1.005F` is exactly
   1.00499999988824…, yet `toString(1.005F)` is `1.01`; likewise `1.015F` → `1.02`, `2.675F` → `2.68`.

Correct behavior (as specified): round the exact stored value half-away-from-zero at every
precision; or, if the decimal-literal-friendly over-rounding of (2) is intended, the spec must say
so — that part is an owner decision. Part (1) is a plain spec/code mismatch either way.

References:

- src/docs/spec/language/18_builtin-functions.md §18.1 note under the table ("`toString` of a `Fixed` renders `decimals` places ... at 10 or more places the extra digits are trailing zeros").
- Related but distinct: bug-818 (rounding skipped near max, fixed); bug-312 K1 (pre-round introduced).

## Failing Reproduction

```
IMPORT io
FUNC main() AS Nothing
  io::print(toString(1.005F))
  io::print(toString(1.005F, toByte(10)))
  io::print(toString(1.005F, toByte(20)))
  LET a = toFixed(1) / toFixed(3)
  io::print(toString(a, toByte(9)))
  io::print(toString(a, toByte(10)))
  io::print(toString(0.125F, toByte(2)))
END FUNC
```

- Observed: `1.01`, `1.0049999998`, `1.00499999988824129104`, `0.333333333`, `0.3333333332`, `0.13`.
- Expected (exact value rounded half-away, computed with Python `Decimal`): `1.00`, `1.0049999999`, `1.00499999988824129105`, `0.333333333`, `0.3333333333`, `0.13`. (The spec's "trailing zeros" would instead give `1.00500000000000000000`-style output, which is also not what happens.)

Contrast: exactly representable midpoints (`0.125F` → `0.13`, `2.5F,0` → `3`, `-2.5F,0` → `-3`) are
right, and so is everything that is not within ~0.5 ULP of a midpoint.

## Root Cause

`src/codegen/string/repr/builder_strings.rs:emit_fixed_to_string_value` (and its constant twin
`src/numeric.rs:fixed_default_to_string_text`, `HALF = (1<<31).div_ceil(100)`):
- `compare_immediate(precision, "10")` / `branch_ge(round_skip)` skips the pre-round entirely for
  n >= 10 on the stated premise that "the Q32.32 fraction (~9.6 decimal digits) is exhausted and the
  remaining places render as trailing zeros" — false: the digit loop (`(frac*10)>>32`) keeps emitting
  the exact binary expansion, so those places are truncated digits.
- The bias is the *ceiling* of half an n-place unit (chosen so the exact `0.125` case rounds up), which
  adds up to one extra raw unit and pushes values just below the midpoint over it. An exact
  half-away test needs the remainder compared against the true half (e.g. compare `frac * 10^n`
  against `2^31` with a 128-bit product, or carry one guard bit).

## Goal

- `toString(x AS Fixed, n)` equals the exact Q32.32 value rounded half-away at n places for every n in 0..255, constant-folded and runtime alike; spec text updated accordingly.

### Non-goals (must NOT change)

- Default precision 2; the bug-818 carry at the range ends; Money/Float renderers.

## Blast Radius

- `fixed_default_to_string_text` (constant fold) — same ceil bias; must change in lockstep (A folded constant renders identically).
- Money renderer it mirrors — not checked here (Money is decimal-scaled, so no binary midpoint issue).
