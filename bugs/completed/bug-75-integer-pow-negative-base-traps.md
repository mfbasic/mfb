# bug-75: integer `^` with a negative base of magnitude >= 2 wrongly traps `ErrOverflow`

Last updated: 2026-07-11
Status: OPEN — never landed. Confirmed still present in the current tree
(builder_numeric.rs:1183-1184: `multiply_registers(dst, left, right)` runs
before `signed_multiply_high_registers(&high, left, right)`; the integer pow
loop calls `emit_checked_integer_multiply(dst, dst, base)` at :1291, so `left ==
dst`). See the root-cause refinement at the bottom — same operand-aliasing
mechanism as bug-74.
Effort: small (<1h)
Severity: MEDIUM (a correct expression fails at runtime)

`emit_integer_pow` (`src/target/shared/code/builder_numeric.rs`) traps `ErrOverflow`
for `(0-2) ^ 3`, whose true value is `-8` and is comfortably representable. The
overflow guard in the repeated-multiply loop appears to compare a signed running
product against an unsigned or magnitude-only bound, so a legitimately negative
intermediate reads as out of range.

The single correct behavior a fix produces: `b ^ e` on `Integer` returns the exact
value whenever it is representable in `i64`, for negative and positive bases alike,
and traps `ErrOverflow` only when it genuinely is not.

## Discovery

Found while fixing bug-61 (commit 7dbf6064), whose non-goals scoped it out. bug-61
added the closed form for base `∈ {-1, 0, 1}`, so `-1 ^ n` is correct; the defect
begins at `|base| >= 2`.

Note the sibling defect in the Fixed operator: bug-74.

## Failing Reproduction

```basic
IMPORT io

FUNC main AS Integer
  LET n AS Integer = 0 - 2
  io::print(toString(n ^ 3))    ' observed: traps ErrOverflow   expected: -8
  RETURN 0
END FUNC
```

Contrast: `2 ^ 3` = 8 works; `(0-1) ^ 3` = -1 works (bug-61's closed form).

## Root Cause

The per-iteration overflow guard in `emit_integer_pow`'s loop does not admit a
negative running product. Compare with the `toInt(text, base)` guard, which bug-49
(commit 1422993e) had to switch from a signed to an unsigned compare for the
mirror-image reason — the two guards should be read together, since the correct
answer differs (this one must admit negatives; that one must not).

## Goal

- Integer `^` is exact for every representable result, and traps only on a true
  overflow.

### Non-goals

- bug-61's `{-1, 0, 1}` closed form (correct; keep it short-circuiting).
- Changing `^` to exponentiation by squaring (bug-61 deliberately did not, since it
  would alter Fixed truncation; the same reasoning applies here for consistency).

## Blast Radius

- `emit_integer_pow` only.

## Fix Design

Work the guard from the magnitude: check `|product| <= i64::MAX / |base|` before
the multiply and carry the sign separately, or use a checked multiply idiom
(`smulh` high-half check) that admits the full signed range. Verify the `i64::MIN`
boundary, which has no positive counterpart.

## Phases

### Phase 1 — failing test

- [ ] `(0-2) ^ 3 == -8`; confirm it traps today.
- [ ] Enumerate the boundary matrix: `(0-2) ^ 62`, `(0-2) ^ 63` (= i64::MIN, exact),
      `(0-2) ^ 64` (must trap), `2 ^ 62`, `2 ^ 63` (must trap).

### Phase 2 — the fix

- [ ] Rework the guard to admit a negative product.

### Phase 3 — validation

- [ ] The boundary matrix above, executed.
- [ ] `scripts/test-accept.sh`.

## Summary

`emit_integer_pow`'s overflow guard rejects negative intermediates, so a negative
base of magnitude two or more traps on results that are perfectly representable.

## Root-cause refinement (goal-02 review, G9, 2026-07-11)

Same operand-aliasing mechanism as bug-74: `emit_integer_pow` loops
`emit_checked_integer_multiply(dst, dst, base)` (builder_numeric.rs:1183-1186),
which computes `dst = dst*base` first, then `high = smulh(dst_new, base)` and
`sign = asr(dst_new, 63)`. For `(0-2)^3`: iteration 1 dst = 1·(−2) = −2; high =
smulh(−2,−2) = 0; sign = −1; 0 ≠ −1 → spurious overflow trap on a representable
result. Positive bases accidentally work because smulh(low,base) and
asr(low,63) agree when everything is small/positive. Fix: make
`emit_checked_integer_multiply` read the original `dst` for the high/sign
computation before overwriting it (or compute into a scratch).
