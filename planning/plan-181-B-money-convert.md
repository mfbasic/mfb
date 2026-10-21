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
| plan-181-A complete: `money::Rate` exists | `target/release/mfb man money types \| grep -c 'money::Rate'` → at least 1 (corrected from `mfb man money \| grep -c '^  *Rate'`, which is 0 even with the record present — plan-181-A Corrections 1) | MET (2026-10-20: `1`) |
| plan-181-A complete: `toRate` exists | `target/release/mfb man money \| grep -c 'toRate'` → at least 2 | MET (2026-10-20: `4`) |

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
- [x] In `src/codegen/builtins/money/gen_money_math.rs`, add `emit_money_rescale(dst, m_raw, numerator, divisor)`. It forms the unsigned 128-bit magnitude of `|m_raw| × |numerator|`, divides it by the unsigned 64-bit magnitude `|divisor|` with a 64-step restoring long division (Corrections 1), rounds once with `emit_apply_rounding_magnitude`, applies the sign, and range-checks. No `checked` parameter: both callers can overflow, so the check is unconditional (Corrections 2).
- [x] The helper's `divisor` of zero is refused before the helper is reached, by the callers (`func_convert.rs:emit_positive_rate_scaled`; `scale`'s divisor is the constant `RATE_SCALE`).
- [x] Do not edit `emit_money_fixed_multiply`, `emit_money_fixed_divide`, `emit_money_multiply` or `emit_money_divide_scalar`. — `git diff -- src/codegen/builtins/money/gen_money_math.rs | grep -c '^-[^-]'` → `0` (102 insertions, 0 deletions).

Acceptance (corrected, Corrections 3): the existing emitters are untouched — `gen_money_math.rs`'s diff is additions only (above) — and the existing money fixtures' run output is unchanged: `scripts/test-accept.sh target/release/mfb /tmp/p181/accept 'money_*'` reports no `build.log` mismatch for any pre-existing money fixture. — MET: the mismatches it reports for pre-existing fixtures are `.ir` only (the plan-181-A companion, A's Corrections 5).

Commit: d13d8bf4a

### Phase B2 — `money::convert`

Tasks:
- [x] Add `src/codegen/builtins/money/func_convert.rs`. The `Body::abi_inline` lowering reads `from.scaled` and `to.scaled` (the word at offset 0 of each record block), refuses a value at or below zero with `ErrInvalidArgument`, calls `emit_money_rescale` with numerator `from.scaled` and divisor `to.scaled`, and checks the result.
- [x] Register it in `src/codegen/builtins/money/mod.rs` with its error declarations (`ErrInvalidArgument`, `ErrOverflow`). Unit test `money::tests::convert_and_scale_lower_inline_and_take_rates`.
- [x] Added: `tests/syntax/money/money_rate_invalid/` also rejects `convert` with a `Fixed` rate, a wrong arity and a non-`Money` value.

Acceptance: `tests/rt-behavior/money/money_convert/` prints these results, each at five places:
- `convert(1000000m, EUR at "1.0842", USD at "1")` → `1084200.00000`
- `convert(10000000000m, USD at "1", JPY at "0.0067")` → `1492537313432.83582`. (The exact value is 10^10 / 0.0067 = 1492537313432.835820895…, rounded at the fifth place to .83582.)
- `convert(1m, USD at "1", USD at "1")` → `1.00000`.
- `convert` with a `to` rate built as `Rate[scaled := 0]` traps `ErrInvalidArgument`.
- `convert` with a `from` rate built as `Rate[scaled := -1]` traps `ErrInvalidArgument`.
- `convert(92233720368547.75807m, USD at "1", USD at "0.5")` traps `ErrOverflow`, because its result is twice the input, which is outside the `Money` range.

MET: the golden run prints `1084200.00000`, `1492537313432.83582`, `1.00000`, then (added cases) `-1084200.00000`, the tie `0.00001` (Commercial) / `0.00000` (Banker), `0.00002` for `0.000015` under Banker, `1.00000` for the largest rate over itself (a 126-bit intermediate), `-92233720368547.75808`, and `error 77050002` for the zero `to` and the `-1` `from`, `error 77050010` for the doubled maximum and for `1m` at 9223372 over 0.000000000001. The traps are caught with `TRAP` and their codes printed, so one program checks every case.

Commit: d13d8bf4a

### Phase B3 — `money::scale`

Tasks:
- [x] Add `src/codegen/builtins/money/func_scale.rs`. It refuses `r.scaled` at or below zero with `ErrInvalidArgument` (shared `emit_positive_rate_scaled`), calls `emit_money_rescale` with numerator `r.scaled` and divisor `RATE_SCALE`, and checks the result.
- [x] Register it in `mod.rs`. Unit test `money::tests::convert_and_scale_lower_inline_and_take_rates`; syntax fixture rows for a `Fixed` rate and a wrong arity.

Acceptance: `tests/rt-behavior/money/money_scale/` prints:
- `scale(100m, toRate("0.07"))` → `7.00000`
- `scale(1.5m, toRate("0.333"))` → `0.49950`
- the tie `scale(0.00001m, toRate("0.5"))` → `0.00001` under `Commercial` and `0.00000` under `Banker`.
- `scale` with `Rate[scaled := 0]` traps `ErrInvalidArgument`.

MET: the golden run prints `7.00000`, `0.49950`, `-0.49950`, `0.00001`, `0.00000`, `0.00000` (negative tie, Banker), `-0.00001` (negative tie, Commercial), `92233720368547.75807` (maximum × 1), `0.00000` (1m × 0.000000000001), `1.00000 error 77050002`, `92233720368547.75807 error 77050010`.

Commit: d13d8bf4a

### Phase B4 — spec and man

Tasks:
- [x] `src/docs/spec/stdlib/13_money.md`: add `convert` and `scale` rows to the "Operation formulas" table, with the formulas `R(m × from / to)` and `R(m × r)` (written `R(m * r / P)`, `P = 10^12`, in the table's raw-carrier terms), and note the single rounding and the range check. Add the invalid-rate rule. — rows, a paragraph citing `emit_money_rescale`, and the **Rates** error item.
- [x] `src/codegen/builtins/money/mod.rs` `MODULE_DESC`: describe `convert` and `scale` in the same style as the existing members; the rounding-mode sentence now names them too.
- [x] Check the description strings against `.ai/man-content.md`. Examples print at five places where the point is the fifth place (`toString(x, toByte(5))`); `scripts/man-run-examples.sh money --run` → `examples: 11 … failed: 0`.

Acceptance: `target/release/mfb spec stdlib money` shows the two new rows; `target/release/mfb man money` lists both functions. — MET: spec lines `money::convert(m, from, to) │ R(m * from / to)…` and `money::scale(m, r) │ R(m * r / P)…`; the man Functions table lists `money::convert` and `money::scale`.

Commit: d13d8bf4a

### Phase B5 — gates

Tasks:
- [x] Run the scoped byte-identity gate: `scripts/artifact-gate.sh target/release/mfb money` → `7 golden(s) checked, 0 diff(s)` (after plan-181-A6's re-baseline, which ran on the A+B compiler).
- [x] Run the scoped acceptance tests: `scripts/test-accept.sh target/release/mfb /tmp/p181/accept 'money_*'` → `acceptance tests passed (15 test(s) ran)`, including `money_convert` and `money_scale`. B added no source to the companion: before the re-baseline, B's build moved no `.ir` that A's had not already moved (`money_rate_*` stayed green across the B rebuild).

Expected: no diff in `tests/byte-identity/money/` beyond what sub-plan A6 already accounted for, and no diff in the existing `money_*` acceptance fixtures. Any unexpected diff is root-caused with `objdump` on one fixture, and the bug is fixed; a golden is never re-baselined to pass.

Commit: d13d8bf4a

## 5. Validation Plan

Landing tier, run once at the end of this sub-plan: B5's two commands. The full suite runs in the background under the project's policy.

## 6. Decisions (owner, recorded as fact)

- `convert` is `R(m × from / to)` in one 128-bit calculation with one rounding.
- `scale` takes a `Rate`, so it can scale only by positive factors. A negative percentage is not expressible; the owner chose the positive `Rate` type.
- Existing `Money` helpers are not refactored; the new helper is added beside them.

## Corrections

1. **`emit_money_fixed_divide` has no 128-bit divide to reuse.** Current State said it "forms the 128-bit numerator and divides by the 64-bit raw value". Read: it divides the 64-bit `|m|` by `|f|` for the integer part and then generates 32 fraction bits with `emit_q32_fraction_bits`, i.e. a 96-bit numerator. `emit_money_rescale` therefore carries its own 64-step restoring division of the full 128-bit product, with the same invariant that makes the existing loop safe (the running remainder stays below a divisor of at most 2^63, so the shift never carries out). A high word at or above the divisor means a quotient of 2^64 or more and is `ErrOverflow` before the loop.
2. **No `checked` parameter.** Both callers can overflow (`scale` by a rate up to 9223372, `convert` by any ratio), so a `checked == false` path would have no caller. The range check is unconditional.
3. **B1's acceptance could not hold as written.** "No diff in the existing `tests/byte-identity/money/` fixtures" is false from plan-181-A onward, because A's source bodies are compiled into every `money` importer (A's Corrections 5). Strengthened to what it was meant to prove: `gen_money_math.rs` changes by additions only, and every pre-existing money fixture's run output is unchanged.
4. **Fixture expectations are printed at five places with `toString(x, toByte(5))`.** `toString(Money)` defaults to two places, which would hide every fifth-place value this plan checks (first run printed `1492537313432.84`).
