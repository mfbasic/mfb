# bug-135 — `Float ^ Float` operator loops once per unit of exponent → effective hang for large exponents

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9).
**Severity:** MED — the `^` operator hangs the program for large whole
exponents (the `math::pow` builtin is unaffected).
**Class:** correctness (non-termination).

## Finding

`src/target/shared/code/builder_numeric.rs:1580-1612` (`emit_float_pow`, loop at
:1602-1607), reached from `emit_float_binary` "^" (:1035-1044). The `^`
operator's float kernel is repeated multiplication with the only exit
`exponent_int == 0`. Unlike the Fixed/Integer loops (bug-61 fix), there is no
early exit: float multiply never traps, and inf·inf / 0·0 keep iterating. A
large whole exponent passes the whole-number domain check (e.g. 1e18 is exactly
representable) and the loop runs 1e18 iterations. Edge: exponent = 2^63 exactly
saturates fcvtzs to i64::MAX yet passes the round-trip check, giving a
9.2e18-iteration loop.

`math::pow` is unaffected (fdlibm kernel) — only the operator spelling.

## Trigger

`2.0 ^ 1.0e18` (or `0.5 ^ 1.0e18`) — program hangs; expected ErrFloatOverflow
(or 0.0) promptly.

## Fix

Route the `^` operator's Float path through the same fdlibm `emit_pow_scalar`
kernel `math::pow` uses (or add inf/zero/±1 early exits and cap the iteration
count), so it terminates.

## Prior art

bug-61 explicitly scoped to Fixed/Integer.
