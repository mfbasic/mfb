# plan-29-F: Money codegen — scale by Float and Fixed

Last updated: 2026-07-07
Effort: medium (1h–2h)

This sub-plan completes Money arithmetic by adding the cases where a Money is scaled by
a `Float` or a `Fixed` — `price * 1.08` (tax rate), `amount * discountFixed`,
`amount / rate`. These are the dimensionally-valid `Money * scalar` / `Money / scalar`
forms where the scalar lives in a fractional domain, so they need fixed-point (for
`Fixed`) and floating (for `Float`) scaling of the raw i64. It depends on plan-29-E
(the `emit_money_binary` dispatcher) and plan-29-D (the rounding mode + shared
`emit_apply_rounding` / its Float twin). Conversions and docs land in plan-29-G.

It complements:

- `./mfb spec language types` (§4.1 numeric edge cases; the exactness caveat for `Money * Float`)

## 1. Goal

- `Money * Float`, `Float * Money`, `Money * Fixed`, `Fixed * Money` → **Money**.
- `Money / Float` → **Money**, `Money / Fixed` → **Money**.
- `Money DIV Float`, `Money DIV Fixed` → **Float**.
- Overflow → `ErrOverflow`; a non-finite `Float` operand or a `Float`/`Fixed` result
  outside Money range → the appropriate error (`ErrInvalidFormat` for a non-finite
  Float operand, `ErrOverflow` for range). Correct on aarch64 and x86-64.

### Non-goals (explicit constraints)

- No new operators or operand orders beyond §1 (the front end already fixed which
  pairings are valid). `scalar / Money`, `Money * Money`, etc. never reach here.
- No conversions (plan-29-G). No new runtime error codes.
- `Money * Float` is **inherently inexact** (Float is inexact) — this is documented,
  not fixed. `Money * Fixed` is exact binary fixed-point scaling.

## 2. Current State

`emit_money_binary` (plan-29-E, `builder_numeric.rs`) dispatches Money `*`/`/`/`DIV`
and currently implements the Integer/Byte/Money operand cases. The Fixed scaling
precedent is `emit_fixed_multiply`/`emit_fixed_divide` (`builder_numeric.rs:1272-1380`):
a 128-bit product/dividend with a `>> 32`/`<< 32` Q32.32 rescale (`smulh`/long
division). `load_numeric_as_double` (`:1431-1459`, gained a Money arm in plan-29-E) and
`load_numeric_as_fixed` (`:1460+`) show operand promotion. The Float multiply/divide
leaf ops are `abi::float_multiply`/`float_divide_d` (`:992-999`).

## 3. Design Overview

Two operand domains added to `emit_money_binary`:

- **Fixed** — exact binary scaling. `Money * Fixed`: 128-bit product `>> 32` back to
  Money scale, finished through `emit_apply_rounding` (mode-aware) with an overflow
  check (like `emit_fixed_multiply` but dividing by 2³² only, not by SCALE).
  `Money / Fixed`: `(raw << 32) / fixed_raw` finished through `emit_apply_rounding`,
  `fixed_raw == 0` → `ErrInvalidArgument`.
- **Float** — inexact. `Money * Float` / `Money / Float`: compute in f64, then round to
  i64 with the current mode (`llround` for `Commercial`, `nearbyint`/round-half-even for
  `Banker` — the Float twin of `emit_apply_rounding`). Guard the Float operand finiteness
  (NaN/Inf → `ErrInvalidFormat`, mirroring `toFixed(Float)`'s check) and the result range
  (non-finite or |result| ≥ 2⁶³ → `ErrOverflow`). `fval == 0` in `/` → `ErrInvalidArgument`
  (non-Float result — decided).

Correctness risk is in the Fixed 128-bit rescale (reuse the vetted `emit_fixed_multiply`
structure, changing only the final scale step) and the Float round/finiteness/range
guards.

## 4. Detailed Design

### 4.1 `Money * Fixed → Money`
`fixed_raw` is Q32.32 (value × 2³²). Product `p = raw * fixed_raw` is scaled by SCALE×2³²;
divide by 2³² to return to Money scale via the 128-bit product (`mul`+`smulh` / `imul`),
finishing the `>> 32` through `emit_apply_rounding` (mode-aware) and overflow-checking
the result into i64 → `ErrOverflow`. Commutative (both orders). Exact for the fixed-point
factor.

### 4.2 `Money / Fixed → Money`
`fixed_raw == 0` → `ErrInvalidArgument`. Else `dividend = raw << 32` (128-bit), signed
divide by `fixed_raw`, finish through `emit_apply_rounding` on the remainder,
overflow-check → `ErrOverflow`. Mirror `emit_fixed_divide`'s long-division, but the
dividend pre-shift is `<< 32` (to cancel the divisor's 2³² scale and keep the Money scale
on `raw`).

### 4.3 `Money * Float → Money` / `Money / Float → Money`
- Finiteness: if the Float operand is NaN/±Inf → `ErrInvalidFormat` (matches the
  `toFixed(Float)` guard at `builder_conversions.rs:738-810`).
- `*`: `r = (raw as f64) * fval`; `/`: `fval == 0` → `ErrInvalidArgument` (decided), else
  `r = (raw as f64) / fval`.
- Round `r` to i64 with the current mode (the Float twin of `emit_apply_rounding`:
  `llround` for `Commercial`, round-half-even for `Banker`); if `r` is non-finite or
  `|r| >= 2^63` → `ErrOverflow`.
- Document the inexactness: `raw` up to ~9.2e18 exceeds f64's 2⁵³ exact-integer range,
  so large `Money * Float` loses low digits — acceptable because a Float rate is itself
  approximate; callers needing exactness use `Money * Fixed` or `Money * Integer`.

### 4.4 `Money DIV Float|Fixed → Float`
Promote both to f64 (`Money`→`raw/100000.0`, `Fixed`→via `load_numeric_as_double`,
`Float`→itself) and divide; result `Float`. Extends the DIV arm from plan-29-E.

### 4.5 Dispatch
Fill the `Float`/`Fixed` operand branches of `emit_money_binary` (both orders for the
commutative `*`; only `Money /` for `/`). No front-end change (plan-29-A already types
these as `Money`/`Float`).

### 4.6 Exactness nudge — `Money * <bare-float-literal>` warning
A new **warn-severity** diagnostic (`2-203-00NN`, e.g. `MONEY_INEXACT_FLOAT_LITERAL`)
that fires **only** when the Float scalar in a Money `*`/`/` is a *bare, suffixless
decimal literal* — `9.99m * 1.08`, `1.08 * 9.99m`, `9.99m / 1.08`. It exists because a
bare decimal defaults to `Float` (`classify_literal`, `src/numeric.rs:34`), so the most
natural way to write a rate silently takes the inexact Float path; a literal rate almost
always wants exact scaling.

**Diagnostic-only — it never changes the type.** `1.08` stays `Float` (context-independent
typing is a deliberate decision, plan-29-A). The warning just annotates the spot and
names both explicit, intent-preserving silences:
- `1.08F` → `Fixed`, **exact** binary fixed-point scaling (the "you probably meant this"
  fix; §4.1's exact path).
- `1.08f` → **explicitly `Float`**, *identical* semantics to bare `1.08` but affirms
  "yes, I want inexact Float scaling here." Warning gone.

The escape hatch is what keeps this a *"say what you mean"* nudge rather than a nag on
intended code: there is always a way to silence it **without** changing behavior (`f`),
in addition to the fix that improves it (`F`).

**Trigger precisely:** at the site where `emit_money_binary` (or the front-end Money-op
check) sees a `Money `*`/`/`` with a Float operand, consult whether that operand's AST
node is a numeric-**literal** node whose lexed text carried **no** `f`/`F` suffix (the
info lives at the syntaxcheck/AST layer, distinct from the resolved `Float` type — thread
"was a suffixless decimal literal" through to the check). Float *variables* never warn
(deliberate). Fixed literals never warn (exact). Both operand orders of `*`, plus
`Money / lit`.

**Message shape:** *"scaling Money by a bare decimal literal uses inexact Float
arithmetic; append `F` (`1.08F`) for exact fixed-point scaling, or `f` (`1.08f`) to
confirm the Float is intentional."*

## Layout / ABI Impact

None — arithmetic only.

## Phases

### Phase 1 — Fixed & Float scaling kernels
`emit_money_binary` handles Float and Fixed operands.

- [ ] `builder_numeric.rs`: `Money * Fixed`, `Money / Fixed` (128-bit `>>32`/`<<32`
      rescale via `emit_apply_rounding`, div-zero → `ErrInvalidArgument`).
- [ ] `Money * Float`, `Money / Float` (finiteness guard → `ErrInvalidFormat`, mode-aware
      round, range → `ErrOverflow`, `/0` → `ErrInvalidArgument`); DIV Float/Fixed → Float.
- [ ] Edge coverage: exact `M * Fixed`; inexact-but-rounded `M * Float`; non-finite
      Float operand; overflow; divide-by-zero; a Fixed/Float tie that rounds differently
      under `Commercial` vs `Banker`.

Acceptance: each case computes the correct rounded raw; the guards fire with the right
error codes.
Commit: —

### Phase 2 — runtime proof
All Money-with-Float/Fixed arithmetic runs correctly.

- [ ] `tests/rt-behavior/**` (observed via plan-29-G print): `9.99m * 1.08` (tax, Float),
      `100.00m * aFixed`, `10.00m / 2.5` (Float), `10.00m / aFixed`, and the failure
      paths (NaN rate, overflow, `/ 0.0`).

Acceptance: correct results on aarch64 and x86-64, verified by an executed program;
the exactness caveat for `Money * Float` holds (documented in plan-29-G).
Commit: —

### Phase 3 — (OPTIONAL, deferrable) `Money * <bare-float-literal>` exactness warning
The diagnostic-only nudge of §4.6. May ship after the type lands — not a blocker for
plan-29 completion.

- [ ] Thread "operand was a suffixless decimal literal" to the Money `*`/`/` check;
      emit `MONEY_INEXACT_FLOAT_LITERAL` (warn) for bare-literal Float scalars only.
- [ ] `rules/table.rs` + `src/docs/spec/diagnostics/01_rule-codes.md`: the new rule.
- [ ] Tests: `_warn` fixture (`9.99m * 1.08`, `9.99m / 1.08`, `1.08 * 9.99m` all warn);
      `_nowarn` (`9.99m * 1.08F`, `9.99m * 1.08f`, `9.99m * aFloatVar`, `9.99m * aFixed`
      — none warn); confirm the type is `Money` in every case (warning changes nothing).

Acceptance: the warning fires on exactly the bare-literal cases and is silenced by both
`F` and `f` suffixes and by a Float variable; the resolved type is unaffected.
Commit: —

## Validation Plan

- Runtime proof: the executed program above on both backends.
- Function tests: land with plan-29-G (print surface).
- Doc sync: §4.1 exactness caveat for `Money * Float` — plan-29-G.
- Acceptance: `scripts/test-accept.sh …`.

## Open Decisions

- **`Money / Float` by zero — DECIDED: `ErrInvalidArgument`** (the result is Money, a
  non-Float, so §4.1's "non-Float divide-by-zero → ErrInvalidArgument" applies —
  pre-checked, not left to a Float boundary). (§4.3)
- **`Money * <bare-float-literal>` warning — DECIDED: add it, as an OPTIONAL/deferrable
  diagnostic-only warning** (§4.6, Phase 3). Fires only on suffixless decimal literals
  (never on Float variables or Fixed literals); silenced by `1.08F` (exact Fixed) **or**
  `1.08f` (explicit Float). Never changes the type — bare-literal typing stays
  context-independent (plan-29-A). The `f`-suffix escape hatch is required so intentional
  Float scaling has a semantics-preserving silence, keeping this a "say what you mean"
  nudge rather than a nag. May ship after the type lands. (§4.6)

## Summary

Completes the dimensional multiply/divide by handling the fractional scalars: exact
128-bit fixed-point scaling for `Fixed`, inexact-but-guarded floating scaling for
`Float`. Reuses the vetted `emit_fixed_multiply`/`_divide` structure with a different
final scale step; the risk is the rescale arithmetic and the Float guards.
