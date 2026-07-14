# bug-230: Money ÷ Integer with an i64::MIN divisor takes the wrong rounding branch

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness

Status: Open

`emit_money_divide_scalar` (`src/target/shared/code/builder_money_math.rs:176-187`)
produces `abs_divisor` via `emit_abs_i64`, which leaves `i64::MIN` unchanged;
`emit_apply_rounding` (`:47-48`) then does signed compares
(`half = abs_divisor - abs_rem`, `branch_lt`/`branch_gt`) that assume a small
positive magnitude, so an `i64::MIN` divisor makes the tie/round logic take the
wrong branch.

Trigger: `someMoney / -9223372036854775808` (Integer divisor = `i64::MIN`). E.g.
`m(i64::MIN raw) / i64::MIN` computes quotient 1, remainder 0, but `half` becomes
negative so `branch_gt(round_up)` fires and the result is raw 2 instead of 1.
(`money::round`'s divisor is always ≤100000, so it is unaffected; only `M / k`
with `k == i64::MIN` reaches this.)

Fix: guard `k == i64::MIN` (or compute the magnitude unsigned as
`emit_fixed_divide` does) before feeding `emit_apply_rounding`.
