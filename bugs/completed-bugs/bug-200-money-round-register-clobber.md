# bug-200: money::round holds the Money raw across the decimals-arg lowering without spilling → register clobber

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: memory-safety (native codegen register lifetime)

Status: Fixed (2026-07-15) — `lower_money_round` now spills the Money raw to a
stack slot before lowering the `decimals` argument and reloads it afterward,
mirroring `lower_math_min_max`/`clamp`/`scalar_binary`, so a `decimals` expression
that emits any `_mfb_*` helper call cannot clobber the raw.
Regression Test: verified at runtime — `money::round(3.14159m, computeDecimals(1))`
(decimals arg is a user call) yields `3.14` (correct rounding); a plain-literal
decimals arg is unaffected.

`lower_money_round` reads the Money raw from a caller-saved register *after*
lowering the `decimals` argument, without spilling it first. If the decimals
expression emits any `_mfb_*` helper call (arena alloc, a function call, etc.),
that call clobbers all caller-saved integer registers per the register-lifetime
model, so the raw is destroyed and the rounded result is wrong. Every sibling in
this file (`lower_math_min_max`, `lower_math_clamp`, `lower_math_rand`,
`lower_math_scalar_binary`) spills the first operand before lowering the second
and warns about exactly this; `lower_money_round` omits it.

## Failing Reproduction

```
money::round(price, computeDecimals())
```
(or any decimals sub-expression whose lowering emits a runtime call). Observed:
the raw is read from a register the helper clobbered → wrong rounded Money.
Expected: correct rounding regardless of what the decimals arg lowers to.

## Root Cause

`src/target/shared/code/builder_money.rs:77-81` `lower_money_round` — `raw =
value.location` (line 80) reads a caller-saved register that may have been
clobbered by the `lower_value(decimals_arg)` call at line 77.

## Non-goals

- Do not change the rounding algorithm or the `money::round` signature.

## Blast Radius

- `lower_money_round` only; the math siblings already spill correctly.

## Fix Design

Spill `value.location` to a stack slot before `lower_value(decimals_arg)`, then
reload after — mirror `lower_math_scalar_binary`.
