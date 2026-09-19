# bug-615: `math::tan` on a `Fixed` near pi/2 silently returns a wrong-signed value instead of `ErrOverflow`

Last updated: 2026-09-13
Effort: small–medium
Severity: MED
Class: Correctness (silent wrong result from checked arithmetic)

Status: Fixed — see STATUS below
Regression Test: none yet — see Phase 1

## STATUS: FIXED (78ec84d2c, adc6e874e, a47fa3534, aa65d431a, 703f18303, 3086dcf5d, ed4c94022, 18025898f)

`Fixed` `sin`, `cos`, `tan` and — after sub-issue C — `asin`, `acos`, `atan`,
`atan2` are within one Q32.32 unit (2^-32) of the true value, measured against an
exact 336-bit oracle: the sin/cos/tan corpus within 0.5007 units, the 855-argument
inverse corpus within 0.4990, i.e. rounding-limited. `tan` raises `ErrOverflow`
when the true tangent leaves the `Fixed` range, declared on the `Fixed` overload
only.

Deviations from the Fix design — hypothesis 1 was wrong, and the scope grew twice:

- **The divide was already range-checked.** `emit_fixed_tan` goes through
  `emit_fixed_divide`, which checks. The wrong value came from a *cosine* that was a
  few units off and wrongly signed, so the quotient was in range.
- **615-B**: the same reduction made `sin`/`cos` wrong at large angles
  (`sin(toFixed(2000000000.0))` was 0.9432 against a true 0.9147) and several units
  off at small ones (`cos(0)` read 1.00000000163).
- **615-C**: `asin`/`acos`/`atan`/`atan2` kept the CORDIC residue (`atan(2.0F)` 4.27
  units). Not optional: once `sin` was correct the errors stopped cancelling and
  `vector::slerp` on a `Fixed2` went from 2.9 units off to 10.9. One exact-ratio
  `atan` kernel now serves all four; the vectoring CORDIC has no callers left and is
  deleted, with the host-`f64` `fixed_pi()`/`fixed_pi_over_2()`.
- **Golden and acceptance values re-derived, each proven**: `sin(math::piFixed)`
  rounds to raw 0, not raw -1 (703f18303), and ~20 pinned `Fixed` expectations moved
  to the correctly rounded value (31d40c93a). The residual spread in the `vector::`
  composites is their own chain — filed as bug-654.

Goldens: 10 `.ncodesum` (math, vector) plus the integration regeneration.

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

### Sub-issue 615-C — the inverse family has the same residue (found 2026-09-19)

Predicted by the blast-radius audit and now measured. `asin`, `acos`, `atan` and
`atan2` on a `Fixed` share the CORDIC **vectoring** loop, which keeps the residue
`sin`/`cos` shed: against the same 336-bit oracle, `atan(2.0F)` is 4.27 units out,
`atan2(3.0F, -4.0F)` 2.69, `asin(0.8F)` 1.61 — outside the one-unit contract this
bug establishes for the family.

It is not cosmetic. `vector::slerp` on a `Fixed2` composes `acos` with `sin`, and
the two errors used to partly cancel: with `sin` correct and `acos` unchanged,
`slerp(Fixed2[3,4], Fixed2[1,2], 0.5).x` moved from 2.9 units off the true value
to 10.9. Leaving 615-C unfixed would land a change that makes a composite worse,
so it is in scope here rather than deferred.

RED test: `fixed_inverse_trig_is_within_one_unit` in
`tests/runtime/rt_math_fixed_trig_accuracy.rs` — 202 wrong results, with an
`atan`/`asin`/`acos`/`atan2` oracle built on argument-halving plus the Taylor
series in 336-bit fixed point.

#### Fixed (2026-09-19)

All four now share **one** kernel, `emit_fixed_atan_ratio(num, den)`: `atan` of a
ratio of two unsigned 64-bit words, returning an unsigned Q3.61 angle in
`[0, pi/2]`. Taking a *ratio* rather than a `Fixed` is the whole fix — the
quotient is never rounded to Q32.32 first, so a huge or a tiny argument keeps its
full relative precision, and both range folds are exact integer rewrites of the
ratio:

- `num > den` → swap the words (`atan t = pi/2 − atan(1/t)`);
- `2·num > den` → `(num, den) := (den − num, den + num)`
  (`atan t = pi/4 − atan((1−t)/(1+t))`).

Both leave `u = num/den <= 1/2`, taken as an exact unsigned Q0.63 quotient by a
63-step long division and fed to a 28-level Horner Taylor series
`atan u = u·Σ(−u²)ⁿ/(2n+1)` in Q1.63 (`FIXED_ATAN_TERMS = 27`; the first omitted
term is `u^55/55 < 2^-60`). The angle carries about 60 bits and the family rounds
**once**, at the Q3.61 → Q32.32 step, so the residual error is rounding-limited.

- `atan2`: axes are exact baked constants (`atan2(0, x<0)` is `+pi`); otherwise
  the quadrant is added in Q3.61 — `x < 0` takes `pi` minus the first-quadrant
  angle, the sign of `y` applied last. `atan(x)` stays `atan2(x, 1.0)`.
- `asin x = atan(x / sqrt(1 − x²))` as an integer ratio. With `x = r/2^32`,
  `1 − x² = d/2^64` for `d = 2^64 − r²`, which is *exactly* the low word of
  `−r·r` because `|r| <= 2^32`; the ratio is `|r|·2^16 / round(sqrt(d)·2^16)`,
  the root being `emit_fixed_sqrt` applied to `d` read as an unsigned word (its
  radicand is `d·2^32`, so its Q32.32 answer *is* `sqrt(d)` with 16 fraction
  bits). **That is how `|x| → 1` is handled**: a Q32.32 `sqrt(1−x²)` keeps one
  or two bits at `x = 1 − 2^-32`, while here the numerator is exact and a
  half-unit root error moves the angle by at most `2^-48` radians
  (`d(atan(r/s))/ds = −r/2^96` at this scale). `|x| = 1` gives `d = 0`, and the
  kernel answers `pi/2` for a zero denominator with no divide.
- `acos x = pi/2 − asin x`, formed in Q3.61 *before* the single rounding step, so
  the two roundings never compound.

Every added constant (`pi`, `pi/2`, `pi/4` in Q3.61; `pi` and `pi/2` in Q32.32)
is `pi_over_2_scaled(shift)`, a `const fn` that rounds the existing baked 160-bit
`PI_OVER_2_Q159` — no second, disagreeing pi, and no host `f64` (bug-137.1).
`trig_constants_match_machin_pi` pins all five against Machin's pi and pins the
series term count.

The 31-step Q32.32 CORDIC vectoring loop lost its last caller and is **deleted**,
with it `CORDIC_ITERATIONS`, `CORDIC_ATAN_TABLE`, `cordic_atan_raw`, and the
host-`f64` `fixed_pi()`/`fixed_pi_over_2()`. (`fixed_raw` survives for the
`exp`/`log` constants.) A stale `money`'s-CORDIC-table citation in
`src/codegen/string/format/float_parse_table.rs` was repointed.

**Measured** (macos-aarch64, `-O0` debug build of an 855-argument corpus — domain
edges, `±1`, `1 − 2^-32`, `i64::MIN`/`i64::MAX` raws, all four `atan2` quadrants
and the axes, and randoms — against the same 336-bit oracle): **0 results over
one unit, maximum error 0.4990 Q32.32 units**, i.e. bounded by the final rounding
alone. Was: `atan(2.0F)` 4.27, `atan2(3.0F, -4.0F)` 2.69, `asin(0.8F)` 1.61.
`vector::slerp(Fixed2[3,4], Fixed2[1,2], 0.5).x` is back to 2.95 units off (it was
2.9 before the `sin` fix and 10.9 after it); the remaining 2.9 is the `Fixed2`
slerp's own intermediate rounding chain, not `acos`.

Gate: `cargo test --test rt_math_fixed_trig_accuracy` — **3 passed**, including
the unchanged `fixed_sin_cos_tan_are_within_one_unit_and_tan_overflow_raises`.
Commits: 615-C kernel + spec, below.

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

- [x] Phase 1 — RED test: `tan(math::pi2Fixed)` raises `ErrOverflow`; locate the
unchecked step; decide the precision question. Commit: 78ec84d2c (RED,
`tests/runtime/rt_math_fixed_trig_accuracy.rs`, 1604 of 1686 results wrong), plus
fe77fba2a (the harness bug the corpus exposed: `run_bounded` deadlocked on any
program printing more than the pipe buffer).

- [x] Phase 2 — rebuild the Q32.32 trig kernel (GREEN); goldens; full suite.
Commit: adc6e874e (kernel), e8df6f608 (fmt), and the docs commit below.

The fix replaces the reduction and the sin/cos evaluation rather than adding a
range check, because the range check was never the defect:

- `emit_fixed_trig_reduce`: `k = round(|x|*2/pi)` from a Q0.64 `umulh`, then
  `|r|*2^127 = |x|*2^127 - k*(pi/2)*2^127` modulo 2^128 against a baked 160-bit
  `pi/2` — reduction error below `2^-127` for every `k < 2^31`.
- `emit_fixed_sin_cos_magnitudes`: a 9-level Horner Taylor series for `sin|r|`
  and `cos|r|` in unsigned Q1.63, rounded once to Q32.32 with the quadrant's sign.
  CORDIC rotation mode is gone; `emit_cordic` becomes vectoring-only for `atan2`,
  whose emitted instructions are unchanged.
- `emit_fixed_tan`: the quotient of those magnitudes, or near a pole (odd
  quadrant, `|r| < 2^-8`) `cot r = 1/r - r/3 - r^3/45` with `1/r` from a 73-step
  128-bit long division — one-unit accuracy at a tangent near `2^31` needs ~95 bits
  of `r`. `ErrOverflow` when the rounded quotient leaves the `Fixed` range.
- Constants are baked integers pinned to Machin's pi by
  `trig_constants_match_machin_pi`; no host `f64` is read at build time, so every
  target agrees bit for bit (verified byte-identical output over a 6162-argument
  corpus on macos-aarch64 at -O0/-O1/-O3, linux-aarch64, linux-x86_64,
  linux-riscv64 and windows-x86_64).
- `ErrOverflow` is declared on the **`Fixed` overload only**, via the new
  `preserving_unary_typed_errors`: the `Float` forms return a large finite value
  near pi/2 and cannot overflow, so declaring it on every overload would put an
  impossible error in the Float rows of the page's Errors table.

Gate: `cargo test --test rt_math_fixed_trig_accuracy` — 2 passed, every result
within 0.5007 units of the 336-bit oracle, no in-range tangent raising.
