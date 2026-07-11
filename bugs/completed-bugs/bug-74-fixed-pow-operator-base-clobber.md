# bug-74: the `Fixed ^ Fixed` operator returns `0.00` for any `|base| >= 2` — `base` is clobbered across `emit_fixed_multiply`

Last updated: 2026-07-11
Status: OPEN — never landed. Confirmed still present in the current tree
(builder_numeric.rs:1315-1316: `multiply_registers(dst, left, right)` runs
before `signed_multiply_high_registers(&high, left, right)`; the pow loop calls
`emit_fixed_multiply(dst, dst, base)` at :1498, so `left == dst` and the
high-part read is poisoned). See the root-cause refinement at the bottom — the
fix is operand-aliasing, not a helper-call spill.
Effort: small (<1h)
Severity: HIGH (silent wrong answer, no trap, on a plain arithmetic operator)

`emit_fixed_pow` (`src/target/shared/code/builder_numeric.rs`) implements the
Fixed `^` operator with a repeated-multiply loop. The loop body calls
`emit_fixed_multiply`, which is a runtime-helper call, and `base` is held in a
caller-saved register across it. Per `.ai/compiler.md` "Native Codegen Register
Lifetimes", every `bl _mfb_*` destroys all of `x0`–`x17`, so `base` comes back
poisoned and the product truncates to zero.

Result: `2.0 ^ 3.0` evaluates to `0.00`. There is no trap and no diagnostic — the
program simply computes the wrong number.

The `math::pow(Fixed, Fixed)` path (`emit_fixed_pow_general` in
`builder_fixed_math.rs`) is **correct**: it keeps `base` in a stack slot across the
multiply. Only the operator spelling is broken.

The single correct behavior a fix produces: `a ^ b` on `Fixed` returns the same
value as `math::pow(a, b)` for every input, or traps identically.

## Discovery

Found while fixing bug-61 (commit 7dbf6064). bug-61's non-goals forbade changing
`|base| >= 2` operator results, so it was left in place; bug-61's `±1.0` fast path
sidesteps it for the bounded-base cases it had to fix, which is why the bug still
hides behind a correct-looking `1.0 ^ n` / `-1.0 ^ n`.

## Failing Reproduction

```basic
IMPORT io
IMPORT math

FUNC main AS Integer
  LET viaOperator AS Fixed = 2.0F ^ 3.0F
  LET viaPow AS Fixed = math::pow(2.0F, 3.0F)
  io::print("operator = " & toString(viaOperator))   ' observed: 0.00   expected: 8.00
  io::print("math::pow = " & toString(viaPow))       ' observed: 8.00
  RETURN 0
END FUNC
```

## Root Cause

`emit_fixed_pow`'s loop keeps `base` in a caller-saved register across the
`emit_fixed_multiply` helper call, which clobbers it. This is the exact class
`.ai/compiler.md` warns about, and the same class as `arena-alloc-clobbers-x14-x15`.

## Goal

- The Fixed `^` operator agrees with `math::pow(Fixed, Fixed)` on every input.

### Non-goals (must NOT change)

- bug-61's `±1.0` closed form and `product == 0` early exit — both are correct and
  should keep short-circuiting before the loop is reached.
- `math::pow`'s existing lowering, which is already correct.

## Blast Radius

- `emit_fixed_pow` only. Audit `emit_integer_pow` for the same pattern (its loop
  calls a multiply too) — see also bug-75, which reports a separate defect there.

## Fix Design

Spill `base` (and the accumulator, if it is likewise live) to a stack slot before
the `emit_fixed_multiply` call and reload after, exactly as `emit_fixed_pow_general`
does. Prefer routing the operator through the already-correct
`emit_fixed_pow_general` if the semantics are identical — that deletes the
duplicate loop rather than fixing it twice.

## Phases

### Phase 1 — failing test

- [ ] Assert `2.0F ^ 3.0F == math::pow(2.0F, 3.0F)`; confirm it returns `0.00` today.
- [ ] Sweep `emit_fixed_pow`/`emit_integer_pow` for every value live across a `bl`.

### Phase 2 — the fix

- [ ] Spill/reload, or unify the operator onto `emit_fixed_pow_general`.

### Phase 3 — validation

- [ ] A property test comparing the operator against `math::pow` across a base and
      exponent matrix, including the overflow boundary.
- [ ] `scripts/test-accept.sh`.

## Summary

The Fixed `^` operator silently returns zero for any base of magnitude two or more,
because its loop trusts a caller-saved register across a runtime-helper call. The
`math::pow` sibling already does it correctly and is the model for the fix.

## Root-cause refinement (goal-02 review, G9, 2026-07-11)

The register-clobber explanation above is likely **wrong**. A fresh trace found
the actual mechanism is **operand aliasing**: `emit_fixed_pow` loops
`emit_fixed_multiply(dst, dst, base)`, and inside `emit_fixed_multiply`
(builder_numeric.rs:1315-1316) `multiply_registers(dst, left, right)` with
`dst == left` overwrites `left` *before* `signed_multiply_high_registers(&high,
left, right)` reads it — so `high` is computed from the low product, and the
`(high<<32)|(dst>>32)` recombine is garbage. For `2.0F^3.0F`, iteration 1's low
product = 2^32·2^33 mod 2^64 = 0 → result 0.00. The repro/severity are
unchanged; the fix should make `emit_fixed_multiply` tolerate `dst == left`
(compute into a scratch first, as `emit_fixed_mul_inplace` at
builder_fixed_math.rs:203 already does and documents), not just spill `base`.
