# bug-615: `math::tan` on a `Fixed` near pi/2 silently returns a wrong-signed value instead of `ErrOverflow`

Last updated: 2026-09-13
Effort: small–medium
Severity: MED
Class: Correctness (silent wrong result from checked arithmetic)

Status: Open
Regression Test: none yet — see Phase 1

`Fixed` is Q32.32 fixed point, and its range ends at about ±2.1e9. The tangent of
a `Fixed` argument just below pi/2 lies far beyond that. MFBASIC's checked
arithmetic says such a result fails with `ErrOverflow` (`mfb man types numeric`).
`math::tan` instead returns a number with the wrong sign and magnitude, and raises
nothing:

```
tan(math::pi2Fixed) -> -1431655767.666667     (Float tan of the same value: 16455215755.102158)
```

`math::pi2Fixed` is `1.570796326734125`, about 6.1e-11 below pi/2, so the true
tangent is about +1.65e10.

It also loses accuracy well before the range is exceeded. `toFloat(x)` is the
same number as `x`, so the Float column is the reference:

| `x` | `math::tan(x)` (Fixed) | `math::tan(toFloat(x))` (Float) |
|---|---|---|
| `toFixed(1.5)` | 14.101420 | 14.101420 |
| `toFixed(1.57)` | 1255.767860 | 1255.765694 |
| `toFixed(1.5707)` | 10381.363377 | 10381.331754 |
| `toFixed(1.57079)` | 158065.924518 | 158058.589083 |
| `toFixed(1.570796)` | 3059093.518519 | 3060704.506850 |
| `toFixed(1.5707963)` | 37025580.198276 | 37262967.896974 |
| `math::pi2Fixed` | **-1431655767.666667** | 16455215755.102158 |
| `toFixed(-1.5707963)` | -37347541.747826 | -37262967.896974 |

**The single correct behavior a fix produces:** `math::tan` on a `Fixed` whose
true tangent is outside the `Fixed` range raises `ErrOverflow`, and it never
returns a value of the wrong sign. Whether the in-range precision loss (0.6% at
1.5707963) is also a defect or is inherent to the Q32.32 algorithm is decided in
Phase 1, and recorded here.

Found by plan-125-C Phase 1's probe of the `math::tan` page claim ("An argument
near an odd multiple of pi/2 can overflow to a non-finite result"). **Filed, not
fixed**, by user instruction during a documentation-only plan ("file all bugs,
make no fixes"). Until it lands, `mfb man math tan` states the current behavior.

## Reproduction

```basic
IMPORT io
IMPORT math

SUB main()
  LET r = toString(math::tan(math::pi2Fixed), toByte(6)) TRAP(e)
    RECOVER "raised " & toString(e.code)
  END TRAP
  io::print(r)
END SUB
```

Observed (macos-aarch64, `worktree-P-125` at `f011d27b9`): `-1431655767.666667`.
Expected: `raised 77050010` (`ErrOverflow`).

The full table comes from `/tmp/p125-ex/tanfixed` (plan-125-C Phase 1 ledger).

## Root cause

Hypotheses, to confirm in Phase 1:

1. The `Fixed` tangent is computed as `sin/cos` in Q32.32, and the quotient's
   range check is missing or only checks the low 64 bits. A quotient near
   1.65e10 wraps: -1431655767.67 is close to -2^32/3 = -1431655765.33.
2. The Q32.32 `cos` near pi/2 is a few ULPs from zero and has lost relative
   precision. That would explain the drift from 1.57 onward, and it stays
   correct in sign until the quotient overflows.

Confirm by locating the `Fixed` overload's body (`grep -rn 'tan' src/codegen/builtins/math/`)
and checking its division for an overflow check.

### Confirmed (Phase 1, 2026-09-17) — hypothesis 1 is wrong, hypothesis 2 is half of it

Reproduced on macos-aarch64 at `798870ec2`: `-1431655767.666667`. **The divide is
not the defect**: `emit_fixed_tan` divides through `emit_fixed_divide`
(`builder_numeric.rs`), which does range-check its quotient. The quotient is simply
in range because the *cosine* is wrong: `sin ≈ 1.0` over `cos = -3` raw units
gives `2^32 / -3 = -1431655765.33`.

The cosine is wrong for two reasons, both in `money/gen_fixed_math.rs:emit_fixed_sincos`:

1. **The quadrant reduction uses pi/2 rounded to Q32.32** (`fixed_pi_over_2()`),
   so `r = theta - k*(pi/2)` is off by `k * 0.26` units. At `math::pi2Fixed` that
   constant *is* the argument, `r` is exactly 0, and the true offset (0.26 units)
   is gone. For large `k` the loss is ruinous: `sin(toFixed(2000000000.0))` is
   0.943211 against a true 0.914710; `cos(-2147483000.5)` is -0.008714 against
   -0.091668.
2. **The 31-step Q32.32 CORDIC leaves a residue of several units** even at tiny
   angles: `cos(0)` = 1.00000000163 (7 units), `sin(0)` = 3 units, `sin(1.0)` 3
   units, `cos(0.5)` 5 units. Near pi/2 that residue decides the sign of the cosine.

So the defect is in `sin` and `cos` as much as in `tan`: **new sub-issue 615-B —
Fixed `sin`/`cos` are several units off at every angle and wildly wrong at large
angles.** It is the Fixed twin of bug-618 (the doc there expected Fixed to be safe
because its range "never reaches the broken region"; the Q32.32 reduction has its
own, worse, broken region).

**Precision decision.** The module's contract (`gen_fixed_math.rs` header) is that
Fixed math is deterministic and rounds to the nearest `Fixed`. The fix therefore
targets: `sin`, `cos`, `tan` within one Q32.32 unit (2^-32) of the true value at
every representable argument, and `tan` raising `ErrOverflow` exactly when its
true value is outside the Fixed range (up to a one-unit band at the boundary).
The in-range precision loss in the filed table (0.6% at 1.5707963) is a defect, not
inherent. RED test: `tests/runtime/rt_math_fixed_trig_accuracy.rs`, against an exact
336-bit big-integer oracle — 1604 of 1686 results fail today.

Blast radius audit: `asin`/`acos` go through `emit_fixed_atan2` (CORDIC vectoring,
same iteration count) — check their accuracy too; `atan2`'s divide-free vectoring
has no range issue but shares the residue.

## Non-goals

- Changing the `Float` overload. It returns a large finite value near pi/2,
  which is correct IEEE behavior at representable arguments.
- Rewording the page to hide the wrong value.

## Blast-radius audit

- Other `Fixed` overloads that divide: `atan2` (Fixed), and any
  `Fixed`-returning member whose true result can exceed ±2^31. Audit each for a
  range check on the final value.
- `Fixed` `/` itself: `mfb man types numeric` says it is checked; confirm the
  trig path does not bypass that check.

## Fix

Phase 1 — RED test: `tan(math::pi2Fixed)` raises `ErrOverflow`; locate the
unchecked step; decide the precision question. Commit:

Phase 2 — add the range check (or route through checked `Fixed` division)
(GREEN); goldens that pin `Fixed` trig; full suite. Commit:
