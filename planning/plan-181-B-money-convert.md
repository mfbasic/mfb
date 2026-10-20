# plan-181-B: money::convert and money::scale

Last updated: 2026-10-19
Effort: large (3h–1d)
Depends on: plan-181-A (the `money::Rate` record and `toRate`)

This sub-plan adds two native functions to the built-in `money` package:
`money::convert(m, from, to)`, which computes `m × from / to` for two rates, and
`money::scale(m, r)`, which computes `m × r`. Both compute the product in 128
bits and round once under the calling thread's `money::Rounding` mode, then check
the result against the `Money` range. Neither uses a binary step.

References:

- `planning/plan-181-A-money-rate.md` (the record and its `scaled` field)
- `src/codegen/builtins/money/gen_money_math.rs` (`emit_money_fixed_multiply`, `emit_money_fixed_divide`, `emit_money_multiply`, `emit_money_divide_scalar`, `emit_apply_rounding_magnitude`)
- `src/docs/spec/stdlib/13_money.md` (the formula table at "Operation formulas", and the rounding rules)
- `.ai/testing-gates.md` (byte-identity and acceptance gates)

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-181-A complete: `money::Rate` exists | `target/release/mfb man money \| grep -c '^  *Rate'` → at least 1 | NOT MET (plan-181-A not yet implemented, 2026-10-19) |
| plan-181-A complete: `toRate` exists | `target/release/mfb man money \| grep -c 'toRate'` → at least 2 | NOT MET (plan-181-A not yet implemented, 2026-10-19) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue.

## 1. Goal

- `money::convert(m, from, to) AS Money` returns `m` expressed in the currency whose rate is `to`, where `from` is the source currency's rate. The value is `R(m × from.scaled / to.scaled)` under the calling thread's mode, checked to the `Money` range.
- `money::scale(m, r) AS Money` returns `R(m × r.scaled / 10^12)` under the mode, checked to the `Money` range.
- For the value 1,000,000 EUR at 1.0842 to USD, `money::convert` returns exactly 1084200.00000.
- For the value 10,000,000,000 USD converted to JPY at rate 0.0067, `money::convert` returns 1492537313432.83582 (the exact result, to five places, is 1492537313432.83582). The current `Fixed` design, which the plan-181 spike measured, overflows at this amount. The new design must succeed.

### Non-goals (explicit constraints)

- **Existing `Money` operations are unchanged.** `Money * Fixed`, `Money / Fixed`, `Money * Integer`, `Money / Integer`, `toMoney(Fixed)` and `toFixed(Money)` keep their current helpers and their current output bytes. Their byte-identity fixtures must not change.
- **`Fixed` is unchanged.**
- **No rounding change.** Rounding is `money::Rounding`, once, on the exact rational result.
- **No new error codes.** The errors used are `ErrInvalidArgument`, `ErrOverflow` and, for the zero or negative rates `Rate` can hold, `ErrInvalidArgument`.

## 2. Current State

- **The 128-bit helper.** `src/codegen/builtins/money/gen_money_math.rs:emit_money_fixed_multiply` forms the full unsigned 128-bit magnitude of `left × right`, drops the 32 low bits as the remainder over 2^32, rounds once and range-checks the signed result. Its doc comment states this.
- **The 128-bit divide.** `gen_money_math.rs:emit_money_fixed_divide` computes `R(m × Q / f)` with a runtime 64-bit divisor `f`: it forms the 128-bit numerator and divides by the 64-bit raw value. The divisor is a runtime value, which is the part `convert` needs.
- **Money by integer.** `gen_money_math.rs:emit_money_multiply` and `emit_money_divide_scalar` are the checked `i64` paths the spec table in `13_money.md` lists as "Money times Integer/Byte: checked `m * k`" and "Money divided by Integer/Byte: `R(m / k)`".
- **Rounding.** `gen_money_math.rs:emit_apply_rounding_magnitude` rounds an unsigned magnitude under the mode. The spec (13_money.md, "Operation formulas") says `R` applies once to the exact rational value.
- **Measured.** A probe project at `/tmp/spike-money-rate` (plan-181 spike) showed the overflow boundary of the current `i64` form. It is outside the repository and is not a gate.

## 3. Design

- **The intermediate.** `m.raw × k` where `m.raw` is the `Money` raw value (an `i64`) and `k` is a rate's `scaled` (an `i64`, at most 2^63) is at most 2^126 in magnitude, so it fits the unsigned 128-bit product. The divisor is the other rate's `scaled` (an `i64`).
- **`convert`.** `R(m.raw × from.scaled / to.scaled)`. The 10^12 factors cancel, so `convert` needs no scale constant. The result is a `Money` raw value, which must fit `i64`.
- **`scale`.** `R(m.raw × r.scaled / 10^12)`, with the divisor the constant `RATE_SCALE`.
- **Rounding.** One rounding of the exact quotient under `money::Rounding`, through the same magnitude rounding rule as `emit_money_fixed_divide`.
- **Range.** The rounded signed quotient is checked against `i64`, the `Money` raw range. Overflow is `ErrOverflow`.
- **Invalid rates.** `from.scaled` and `to.scaled` must each be above zero; otherwise `ErrInvalidArgument`. `scale`'s `r.scaled` must be above zero as well. The check is on each call, as decided in plan-181-A.
- **Zero rate.** A `to` rate with `scaled` zero is the zero divisor, and is covered by the rule above.

### Rejected alternatives

- **Generalising `emit_money_fixed_multiply` and `emit_money_fixed_divide` in place.** Rejected: both are used by existing byte-identity fixtures, and changing their emitted code risks changing existing output. The new function calls the division routine, but the existing emitters are not edited.
- **Two calls, `scale` then `scale`.** Rejected: two roundings, which can differ from the one-rounding exact result in the last place.
- **Applying the scale of 10^12 as a division of a `Money` by an `Integer` rate.** Rejected: the rate is not an `Integer`, and the 10^12 divide would round before the multiply.

## 4. Phases

### Phase B1 — the shared 128-bit rescale helper

Tasks:
- In `src/codegen/builtins/money/gen_money_math.rs`, add `emit_money_rescale(dst, m_raw, numerator, divisor, checked)`. It forms the unsigned 128-bit magnitude of `|m_raw| × |numerator|`, divides it by the unsigned 64-bit magnitude `|divisor|` using the division routine that `emit_money_fixed_divide` uses, rounds once with `emit_apply_rounding_magnitude`, applies the sign, and range-checks when `checked` is set.
- The helper's `divisor` of zero is refused before the helper is reached, by the callers.
- Do not edit `emit_money_fixed_multiply`, `emit_money_fixed_divide`, `emit_money_multiply` or `emit_money_divide_scalar`.

Acceptance: `scripts/artifact-gate.sh target/release/mfb money` shows no diff in the existing `tests/byte-identity/money/` fixtures, which proves the existing emitters are untouched.

Commit: —

### Phase B2 — `money::convert`

Tasks:
- Add `src/codegen/builtins/money/func_convert.rs`. The lowering reads `from.scaled` and `to.scaled`, refuses a value at or below zero with `ErrInvalidArgument`, calls `emit_money_rescale` with numerator `from.scaled` and divisor `to.scaled`, and checks the result.
- Register it in `src/codegen/builtins/money/mod.rs` with its error declarations.

Acceptance: `tests/rt-behavior/money/money_convert/` prints these results, each at five places:
- `convert(1000000m, EUR at "1.0842", USD at "1")` → `1084200.00000`
- `convert(10000000000m, USD at "1", JPY at "0.0067")` → `1492537313432.83582`. (The exact value is 10^10 / 0.0067 = 1492537313432.835820895…, rounded at the fifth place to .83582.)
- `convert(1m, USD at "1", USD at "1")` → `1.00000`.
- `convert` with a `to` rate built as `Rate[scaled := 0]` traps `ErrInvalidArgument`.
- `convert` with a `from` rate built as `Rate[scaled := -1]` traps `ErrInvalidArgument`.
- `convert(92233720368547.75807m, USD at "1", USD at "0.5")` traps `ErrOverflow`, because its result is twice the input, which is outside the `Money` range.

Commit: —

### Phase B3 — `money::scale`

Tasks:
- Add `src/codegen/builtins/money/func_scale.rs`. It refuses `r.scaled` at or below zero with `ErrInvalidArgument`, calls `emit_money_rescale` with numerator `r.scaled` and divisor `RATE_SCALE`, and checks the result.
- Register it in `mod.rs`.

Acceptance: `tests/rt-behavior/money/money_scale/` prints:
- `scale(100m, toRate("0.07"))` → `7.00000`
- `scale(1.5m, toRate("0.333"))` → `0.49950`
- the tie `scale(0.00001m, toRate("0.5"))` → `0.00001` under `Commercial` and `0.00000` under `Banker`.
- `scale` with `Rate[scaled := 0]` traps `ErrInvalidArgument`.

Commit: —

### Phase B4 — spec and man

Tasks:
- `src/docs/spec/stdlib/13_money.md`: add `convert` and `scale` rows to the "Operation formulas" table, with the formulas `R(m × from / to)` and `R(m × r)`, and note the single rounding and the range check. Add the invalid-rate rule.
- `src/codegen/builtins/money/mod.rs` `MODULE_DESC`: describe `convert` and `scale` in the same style as the existing members.
- Check the description strings against `.ai/man-content.md`.

Acceptance: `target/release/mfb spec stdlib money` shows the two new rows; `target/release/mfb man money` lists both functions.

Commit: —

### Phase B5 — gates

Tasks:
- Run the scoped byte-identity gate: `scripts/artifact-gate.sh target/release/mfb money`.
- Run the scoped acceptance tests: `scripts/test-accept.sh target/release/mfb <outdir> 'money_*'`.

Expected: no diff in `tests/byte-identity/money/` beyond what sub-plan A6 already accounted for, and no diff in the existing `money_*` acceptance fixtures. Any unexpected diff is root-caused with `objdump` on one fixture, and the bug is fixed; a golden is never re-baselined to pass.

Commit: —

## 5. Validation Plan

Landing tier, run once at the end of this sub-plan: B5's two commands. The full suite runs in the background under the project's policy.

## 6. Decisions (owner, recorded as fact)

- `convert` is `R(m × from / to)` in one 128-bit calculation with one rounding.
- `scale` takes a `Rate`, so it can scale only by positive factors. A negative percentage is not expressible; the owner chose the positive `Rate` type.
- Existing `Money` helpers are not refactored; the new helper is added beside them.
