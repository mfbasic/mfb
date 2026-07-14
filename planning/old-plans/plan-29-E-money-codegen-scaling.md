# plan-29-E: Money codegen — scale by Integer/Byte, M/M ratio, M MOD M

Last updated: 2026-07-07
Effort: medium (1h–2h)

This sub-plan adds the Money arithmetic whose operands stay in the **integer domain**:
scaling a Money by an `Integer`/`Byte` (`price * qty`, `total / count`), the
`Money / Money → Float` ratio, and `Money MOD Money → Money`. It also introduces the
central `emit_money_binary` dispatcher that plan-29-F extends for Float/Fixed operands.
It depends on plan-29-C (Money storage + add/sub/compare) and plan-29-D (the rounding
mode + the shared `emit_apply_rounding` helper that `M / k` consults). Money↔Float and
Money↔Fixed scaling land in plan-29-F; conversions in plan-29-G. Per the plan-29-C
decision, C/D/E land together.

It complements:

- `./mfb spec language types` (§4.1 numeric edge cases — Money overflow / divide-by-zero)

## 1. Goal

- `Money * Integer`, `Integer * Money`, `Money * Byte`, `Byte * Money` → **Money**,
  exact (`raw * k`), overflow-checked.
- `Money / Integer`, `Money / Byte` → **Money**, rounded via the current mode
  (`emit_apply_rounding`, plan-29-D — default half-away), `k == 0` → `ErrInvalidArgument`.
- `Money / Money` → **Float** (the value ratio `raw_a / raw_b`).
- `Money MOD Money` → **Money** (`raw_a MOD raw_b`), `raw_b == 0` → `ErrInvalidArgument`.
- `Money DIV Money`, `Money DIV Integer/Byte` → **Float**.
- Results correct on aarch64 and x86-64.

### Non-goals (explicit constraints)

- No Money↔Float / Money↔Fixed operands (plan-29-F). No conversions (plan-29-G).
- The front end (plan-29-A) already rejected `Money * Money`, `Money ± scalar`,
  `scalar / Money`, `Money MOD scalar`, `Money ^ n` — codegen never sees them and adds
  no runtime guard for them.
- No new runtime error codes: reuse `ErrOverflow` (77050010) and `ErrInvalidArgument`
  (77050002, non-Float divide/MOD by zero, per §4.1). `Money / Money → Float` divide by
  zero is **not** pre-checked — it yields ±Inf/NaN caught at the Float observation
  boundary (`ErrFloatOverflow`/`ErrFloatNaN`), exactly like every other Float `/`.

## 2. Current State

`lower_arithmetic_binary` (`builder_numeric.rs:88`, switch `:183-227`) routes by result
type; plan-29-C added the Money `+`/`-`→`emit_integer_binary` arm. The Fixed path
(`emit_fixed_binary` `:915-960`) shows the shape of a per-type binary dispatcher and
its mixed-operand promotion (`load_numeric_as_fixed` `:1460+`, `load_numeric_as_double`
`:1431-1459`). The checked integer multiply/divide/MOD emitters already exist (used by
`emit_integer_binary` `:830`) — Money `* / MOD` by an Integer/Byte reuse them on the raw
i64 with the scale accounted for as below.

## 3. Design Overview

Add `emit_money_binary(op, left, right, left_type, right_type)` to `builder_numeric.rs`,
dispatched from the result-type switch whenever the result type is `Money`/`Float` and a
Money operand is present with `op ∈ {*, /, MOD, DIV}`. In this sub-plan it handles the
cases where the *other* operand is `Integer`/`Byte` (or the operand is Money itself);
plan-29-F adds `Float`/`Fixed` branches to the same function. The math is plain i64 (and
one i64→f64) — no 128-bit kernel is needed until the exact `M * M` case (which is a type
error) or the Fixed scaling (plan-29-F). Correctness risk is small: rounding direction on
`M / k`, and the zero-divisor error code (`ErrInvalidArgument` for the Money results,
Float-boundary for the ratio).

## 4. Detailed Design

`raw` is the Money operand's i64 (value × 100000); `k` is the Integer/Byte operand's i64.

### 4.1 `Money * Integer|Byte → Money`  (`emit_money_binary`, `*`)
`raw_out = raw * k`, using the existing checked-integer multiply (overflow →
`ErrOverflow`). Exact: scaling an amount by an integer count. Commutative — handle both
operand orders.

### 4.2 `Money / Integer|Byte → Money`  (`/`)
`k == 0` → `ErrInvalidArgument`. Else compute the signed truncating divide (quotient +
remainder) and finish through **`emit_apply_rounding(quotient, rem, |k|, sign)`**
(plan-29-D) so the current mode decides the half case (default half-away; Banker →
half-even). No 128-bit math (dividing an i64 by an i64). Only the `Money / scalar`
direction exists (front end blocked `scalar / Money`).

### 4.3 `Money / Money → Float`  (`/`, both Money)
`result = (raw_a as f64) / (raw_b as f64)` — the SCALE cancels, so this is exactly the
value ratio. Result is `Float`; div-by-zero follows Float rules (no pre-check; ±Inf/NaN
at the boundary). Add a `Money` arm to `load_numeric_as_double` producing `raw as f64 /
100000.0` for the DIV/ratio paths **and** for plan-29-G's `toFloat(Money)`.

### 4.4 `Money MOD Money → Money`  (`MOD`, both Money)
The scale cancels (`a MOD b = (raw_a MOD raw_b)/SCALE`), so `raw_out = raw_a MOD raw_b`
via the existing checked-integer MOD. `raw_b == 0` → `ErrInvalidArgument`.

### 4.5 `DIV` (Money with Money / Integer / Byte) → Float
`DIV` is forced-Float division: promote both operands to f64 (`Money`→`raw/100000.0`,
`Integer`/`Byte`→`value as f64`) and divide. `Money DIV Money`, `Money DIV Integer`,
`Money DIV Byte` all → `Float`. (`scalar DIV Money` was rejected by the front end.)

### 4.6 Dispatch wiring
`builder_numeric.rs` result-type switch: when a Money operand is present and
`op ∈ {*, /, MOD, DIV}`, call `emit_money_binary`, which matches on the (op, operand
types) and emits §4.1–4.5. Leave a clear `todo!`-free fallthrough for the Float/Fixed
operand types that plan-29-F fills — but since E lands right after, wire the dispatch to
cover all Money `*`/`/` operand types and implement the Integer/Byte/Money cases now.

## Layout / ABI Impact

None — arithmetic only.

## Phases

### Phase 1 — dispatcher + integer-domain kernels
`emit_money_binary` with the Integer/Byte, ratio, MOD, and DIV cases.

- [ ] `builder_numeric.rs`: `emit_money_binary` + dispatch wiring; `M * k`, `M / k`
      (via `emit_apply_rounding`, div-zero → `ErrInvalidArgument`), `M / M → Float`,
      `M MOD M`, DIV cases (§4.1–4.5); `Money` arm in `load_numeric_as_double`.
- [ ] Edge coverage: max-magnitude `M * k` → `ErrOverflow`; `M / 0` →
      `ErrInvalidArgument`; `M MOD 0` → `ErrInvalidArgument`; `M / M` rounding + the
      div-by-zero-at-boundary Float path; a `M / k` tie that rounds differently under
      `Commercial` vs `Banker` (proves the mode is consulted).

Acceptance: unit/edge behavior of each case is correct; the dispatch handles both
operand orders for `*`.
Commit: —

### Phase 2 — runtime proof
Every integer-domain Money operation runs correctly end-to-end.

- [ ] `tests/rt-behavior/**` (observed via plan-29-G print, or a raw-equality harness):
      `2.50m * 3` = `7.50000`, `10.00m / 4` = `2.50000`, `1.00m / 3` = `0.33333`
      (round), `10.00m / 4.00m` = `2.5` (Float ratio), `7.00m MOD 3.00m` = `1.00000`,
      plus the overflow / divide-by-zero failure paths.

Acceptance: correct raw results and error codes on aarch64 and x86-64, verified by an
executed program.
Commit: —

## Validation Plan

- Runtime proof: the executed program above on both backends.
- Function tests: land with the conversions in plan-29-G (the print surface).
- Doc sync: §4.1 Money overflow/divide-by-zero — plan-29-G doc sweep.
- Acceptance: `scripts/test-accept.sh …`.

## Open Decisions

- **Rounding — DECIDED: default round-half-away-from-zero (`Commercial`), runtime-
  switchable to round-half-to-even (`Banker`) via `money::setRounding` (plan-29-D).**
  Every Money rounding site goes through the shared `emit_apply_rounding` helper. (§4.2)

## Summary

The integer-domain half of Money arithmetic: exact integer scaling, a Float ratio, and
a remainder — all reusing the checked-integer emitters plus one i64→f64. It stands up
the `emit_money_binary` dispatcher that plan-29-F extends. No 128-bit kernels here; the
risk is only rounding and the zero-divisor error code.
